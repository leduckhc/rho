//! The `rho sessions` subcommands.
//!
//! See `SPEC-session-store-wiring` sections 8, 8a, 8b, and 8e.
//!
//! This module resolves a session, then acts. The printing lives in `sessions`, so a test can
//! read the exact text a user sees without a store, and this module holds the store work.
//!
//! **Every action here is read-only, except `delete`, `fork`, and `name`.** None of them sends
//! anything to a model, so looking at yesterday's session costs nothing. A review found that a
//! user could not simply look, because `run` needs a prompt. See section 8e.

use std::path::Path;

use rho_core::{
    PrefixMatch, RecordId, SessionError, SessionId, SessionLog, SessionReader, SessionRecorder,
    SessionRow, SessionStore,
};

use crate::cli::SessionsAction;
use crate::recording::store_for;
use crate::sessions::{render_list, render_show};

/// Run one action, and return the text to print.
///
/// The text is returned rather than printed, so a test reads it. `home` and `now_millis` are
/// parameters for the same reason: a test never reads the real home directory and never sleeps.
pub fn run(
    home: &Path,
    project_root: &Path,
    action: &SessionsAction,
    now_millis: u64,
) -> anyhow::Result<String> {
    let (store, _key) = store_for(home, project_root);
    match action {
        SessionsAction::List { long } => {
            let rows = store.rows()?;
            if rows.is_empty() {
                return Ok("no sessions in this project yet.\n".to_string());
            }
            Ok(render_list(&rows, now_millis, *long))
        }
        SessionsAction::Show { id, full } => {
            let id = resolve(&store, id)?;
            // A read takes no lock, so `show` works on a live session. See section 7d.
            let read = SessionReader::read(&store.path_of(&id))?;
            Ok(render_show(&read, &id, *full))
        }
        SessionsAction::Delete { id } => {
            let id = resolve(&store, id)?;
            store.delete(&id)?;
            Ok(format!(
                "deleted session {}. A session forked from it is its own file, so it stays. The \
                 bytes are not overwritten, so a recovery tool may still find them.\n",
                id.as_str()
            ))
        }
        SessionsAction::Fork { id, at } => {
            let id = resolve(&store, id)?;
            let record = RecordId(at.trim().to_string());
            // A fork writes a new file, so it takes the lock on what it writes. The source is
            // only read, and it is left byte-identical.
            let new_id = mint_free_id(&store, now_millis)?;
            let _lock = store.lock(&new_id)?;
            let writer = store.fork(&store.path_of(&id), &record, &new_id)?;
            Ok(format!(
                "forked session {} at record {record} into {}.\n{}\n",
                id.as_str(),
                new_id.as_str(),
                writer.path().display()
            ))
        }
        SessionsAction::Name { id, title } => {
            let id = resolve(&store, id)?;
            // A name is a write, so it takes the lock. A live session refuses, because two
            // writers would interleave their lines.
            let _lock = store.lock(&id)?;
            let writer = store.append_to(&store.path_of(&id))?;
            let mut recorder = SessionRecorder::new(SessionLog::File(writer));
            recorder.record_name(title)?;
            Ok(format!(
                "named session {} {:?}.\n",
                id.as_str(),
                title.trim()
            ))
        }
    }
}

/// Resolve a prefix to exactly one session, or refuse.
///
/// An ambiguous prefix lists every match. **A prefix never picks one**, because a resume replays
/// a whole conversation to a model and the wrong one is expensive.
fn resolve(store: &SessionStore, prefix: &str) -> anyhow::Result<SessionId> {
    match store.resolve_prefix(prefix.trim())? {
        PrefixMatch::One(id) => Ok(id),
        PrefixMatch::None => Err(SessionError::NoSuchSession {
            prefix: prefix.trim().to_string(),
        }
        .into()),
        PrefixMatch::Many(matches) => Err(SessionError::AmbiguousPrefix {
            prefix: prefix.trim().to_string(),
            matches: matches.iter().map(|id| id.as_str().to_string()).collect(),
        }
        .into()),
    }
}

/// Mint an id no session in this store already holds.
///
/// A fork names its own new file, so it needs a free id before it writes. The store's own
/// `create_minted` cannot be used, because a fork writes the copied branch instead of a header
/// and a model record.
fn mint_free_id(store: &SessionStore, now_millis: u64) -> anyhow::Result<SessionId> {
    let taken: Vec<SessionId> = store
        .rows()?
        .into_iter()
        .filter_map(|row| match row {
            SessionRow::Session(summary) => Some(summary.id),
            SessionRow::Unreadable { .. } => None,
        })
        .collect();
    for attempt in 0..rho_core::MINT_ATTEMPTS {
        let suffix = ((now_millis >> 3).wrapping_add(attempt as u64 * 7919) % 65_536) as u16;
        let id = SessionId::mint(now_millis, suffix);
        if !taken.contains(&id) {
            return Ok(id);
        }
    }
    Err(SessionError::MintExhausted {
        attempts: rho_core::MINT_ATTEMPTS,
    }
    .into())
}
