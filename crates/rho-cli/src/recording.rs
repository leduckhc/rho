//! Where a run's session file comes from, and how a resume rebuilds a conversation.
//!
//! See `SPEC-session-store-wiring` sections 7, 7d, 8, and 9, and the decisions
//! `D-session-store-layout`, `D-resume-never-widens`, `D-a-live-session-holds-a-lock`, and
//! `D-a-stale-result-handle-expires-on-resume`.
//!
//! Before this module rho wrote no session file at all. `SessionStore` existed, and no
//! production caller reached it. See `D-no-caller-writes-a-session-file`.

use std::path::{Path, PathBuf};

use rho_core::{
    AgentEvent, ContentBlock, Message, NewSession, NewSessionWithoutId, PrefixMatch, ProjectKey,
    Record, SessionError, SessionId, SessionLock, SessionLog, SessionReader, SessionRecorder,
    SessionStore, StoredApproval, StoredSandbox, branch_messages, check_resume_permission,
    default_store_root, expire_stale_result_handles,
};

/// Which session a run uses. The command line resolves exactly one of these.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionSelector {
    /// A fresh session. The default.
    New,
    /// The newest session for this project key.
    Newest,
    /// One session, named by an id or a prefix of one.
    Named(String),
}

impl SessionSelector {
    /// Read the selector from the one session flag.
    ///
    /// `--continue` and `--resume` are the same clap argument with two spellings, so they can
    /// never disagree and `--continue --resume=<id>` is refused by clap itself. See section 8.
    ///
    /// `None` means the flag was absent. An empty value is the bare flag, because clap fills a
    /// missing value with the empty string. Any other value is the named form.
    pub fn from_flag(flag: Option<&str>) -> Self {
        match flag {
            None => SessionSelector::New,
            Some(value) if value.trim().is_empty() => SessionSelector::Newest,
            Some(id) => SessionSelector::Named(id.trim().to_string()),
        }
    }

    /// Does this selector open an existing session?
    pub fn resumes(&self) -> bool {
        !matches!(self, SessionSelector::New)
    }
}

/// Does this text have the shape of a session id?
///
/// `<YYYYMMDD-HHMMSS>-<4 hex>` is the whole id, and a user types a prefix of it. So a prefix of
/// at least the date part counts here.
fn looks_like_a_session_id(text: &str) -> bool {
    let text = text.trim();
    if text.len() < 8 || text.len() > 20 {
        return false;
    }
    let bytes = text.as_bytes();
    bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[8..]
            .iter()
            .all(|b| b.is_ascii_hexdigit() || *b == b'-')
}

/// Refuse a prompt that is a session id, and name the flag the user meant.
///
/// This closes a **measured** fail-open case. `rho run --resume 20260825-09` parses as a bare
/// flag plus a positional prompt, so rho continued the newest session and sent the id to the
/// model as a question. No error appeared. See section 8c.
///
/// rho never guesses which one the user meant. The refusal is the whole rule.
pub fn refuse_an_id_shaped_prompt(prompt: &str, selector: &SessionSelector) -> anyhow::Result<()> {
    if matches!(selector, SessionSelector::Newest) && looks_like_a_session_id(prompt) {
        return Err(anyhow::anyhow!(
            "the prompt {prompt:?} looks like a session id. The value needs an equals sign, so \
             write --resume={prompt} and give the prompt after it."
        ));
    }
    Ok(())
}

/// What a run needs to open its session file.
#[derive(Clone, Debug)]
pub struct RecordingRequest<'a> {
    /// The project root. It gives the project key, so every worktree of one repository shares
    /// one pool of sessions.
    pub project_root: &'a Path,
    /// The home directory the store sits under. It is a parameter, so a test never reads the
    /// real home directory.
    pub home: &'a Path,
    /// One exact file, from the `session-file` key. It overrides the store.
    pub session_file: Option<&'a Path>,
    /// True when the run writes no file at all.
    pub ephemeral: bool,
    pub selector: SessionSelector,
    pub allow_widen: bool,
    /// The resolved mode names this run will use. They go into a new header, and a resume
    /// compares them against the stored ones.
    pub approval: &'a str,
    pub sandbox: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    /// The wall clock, as a parameter, so a test mints a known id and never sleeps.
    pub now_millis: u64,
}

/// The session a run writes, and the conversation it starts from.
pub struct Recording {
    /// The recorder that folds the event stream into records.
    pub recorder: SessionRecorder,
    /// The messages a resume replays. Empty for a new session.
    pub messages: Vec<Message>,
    /// Lines to show the user once.
    pub notices: Vec<String>,
    /// The session this run writes, or `None` when the run is ephemeral.
    pub id: Option<SessionId>,
    /// The file this run writes, or `None` when the run is ephemeral.
    pub path: Option<PathBuf>,
    /// The advisory lock. Holding it stops a second process writing this session.
    ///
    /// It must live as long as the recorder. Dropping it early would let another rho open the
    /// same file, and both would mint the same record ids.
    _lock: Option<SessionLock>,
}

impl Recording {
    /// A recording that degraded to ephemeral, with the reason.
    ///
    /// A session file is not worth ending a run for. See `D-write-failure-degrades`.
    pub fn degraded(reason: String) -> Self {
        Self::ephemeral(vec![reason])
    }

    /// Close the session, so the file states its own close.
    ///
    /// A cancel keeps the session open instead, per `D-cancel-keeps-the-session-open`. So only a
    /// run that ended on its own calls this.
    pub fn close(&mut self) {
        if let Err(error) = self.recorder.close() {
            tracing::warn!(%error, "the session file could not state its close");
        }
    }

    /// Record the prompt, so the file states what the user asked before the answer arrives.
    ///
    /// The three lifecycle calls live here, and not at the call site, because a caller that
    /// forgot one would leave a session with no prompt, no answer, or no close. rho already has
    /// a defect class for a guard whose caller forgot to wire it. See
    /// `D-plugin-does-not-classify-itself`.
    pub fn start(&mut self, input: &[ContentBlock]) {
        self.recorder.record_prompt(input);
    }

    /// Fold one event into the records.
    pub fn observe(&mut self, event: &AgentEvent) {
        self.recorder.observe(event);
    }

    /// An ephemeral recording. It writes nothing.
    fn ephemeral(notices: Vec<String>) -> Self {
        Self {
            recorder: SessionRecorder::new(SessionLog::Off),
            messages: Vec::new(),
            notices,
            id: None,
            path: None,
            _lock: None,
        }
    }
}

/// The store for one project.
///
/// The key comes from the git common directory, so every worktree of one repository shares one
/// pool. rho parses the `.git` entry itself and spawns no git process.
pub fn store_for(home: &Path, project_root: &Path) -> (SessionStore, ProjectKey) {
    let key = ProjectKey::resolve(project_root);
    let root = default_store_root(home).join(key.as_str());
    (SessionStore::new(root), key)
}

/// Open the session this run records into.
///
/// A new run creates a file. A resume reopens one, checks the stored modes, and rebuilds the
/// conversation. An ephemeral run writes nothing.
pub fn open(request: RecordingRequest<'_>) -> anyhow::Result<Recording> {
    if request.ephemeral {
        if request.selector.resumes() {
            return Err(anyhow::anyhow!(
                "--ephemeral writes no file, so there is nothing to continue. Drop one of the two."
            ));
        }
        return Ok(Recording::ephemeral(vec![
            "this session is ephemeral, so rho writes no session file.".to_string(),
        ]));
    }
    let (store, key) = store_for(request.home, request.project_root);
    if request.selector.resumes() {
        return resume(&store, &key, request);
    }
    create(&store, request)
}

/// Create a fresh session, and mint a free id.
fn create(store: &SessionStore, request: RecordingRequest<'_>) -> anyhow::Result<Recording> {
    let new = NewSessionWithoutId {
        cwd: request.project_root,
        approval: request.approval,
        sandbox: request.sandbox,
        provider: request.provider,
        model: request.model,
        forked_from: None,
    };
    // `session-file` names one exact file, so it does not go through the store's own naming.
    let (id, writer) = match request.session_file {
        Some(path) => {
            let id = SessionId::mint(request.now_millis, suffix_from(request.now_millis));
            let named = NewSession {
                id: &id,
                cwd: new.cwd,
                approval: new.approval,
                sandbox: new.sandbox,
                provider: new.provider,
                model: new.model,
                forked_from: None,
            };
            // An existing named file is reopened, because the key names one file for every run.
            if path.exists() {
                let writer = store_append(path)?;
                (id, writer)
            } else {
                let writer = SessionStore::create_file(path, named)?;
                (id, writer)
            }
        }
        None => store.create_minted(request.now_millis, new)?,
    };
    let path = writer.path().to_path_buf();
    // The lock goes on the file the run writes. A create is exclusive, so the window between
    // the create and the lock cannot lose a session; the lock stops a **later** process.
    let lock = match request.session_file {
        // A named file lives outside the store's naming, so the store cannot name its lock.
        Some(_) => None,
        None => Some(store.lock(&id)?),
    };
    Ok(Recording {
        recorder: SessionRecorder::new(SessionLog::File(writer)),
        messages: Vec::new(),
        notices: Vec::new(),
        id: Some(id),
        path: Some(path),
        _lock: lock,
    })
}

/// Reopen a named file, whatever its name.
fn store_append(path: &Path) -> Result<rho_core::SessionWriter, SessionError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    SessionStore::new(parent).append_to(path)
}

/// A four hex suffix for a named file. It never names the file, so it only fills the header.
fn suffix_from(now_millis: u64) -> u16 {
    (now_millis % 65_536) as u16
}

/// Reopen an existing session, and rebuild its conversation.
fn resume(
    store: &SessionStore,
    key: &ProjectKey,
    request: RecordingRequest<'_>,
) -> anyhow::Result<Recording> {
    let id = target(store, key, &request.selector)?;
    // The lock comes before the read. Two worktrees share one project key, so two runs can
    // reach one file, and both would seed their record ids from the same read.
    let lock = store.lock(&id)?;
    let path = store_path(store, &id);
    let read = SessionReader::read(&path)?;

    // A stored mode may only **tighten** this run. It can never widen it, and it never stands
    // in for --allow-widen. See `D-resume-never-widens`.
    check_resume_permission(
        &read.header,
        StoredApproval::parse(request.approval),
        StoredSandbox::parse(request.sandbox),
        request.allow_widen,
    )?;

    let head = read
        .entries
        .last()
        .map(|entry| entry.id.clone())
        .unwrap_or_else(|| read.header_id.clone());
    let mut messages = branch_messages(&read.entries, &head, Some(&read.header_id))?;
    // The store behind an old result handle died with the earlier run, so the promise expires.
    expire_stale_result_handles(&mut messages);

    let mut notices = vec![format!(
        "continuing session {} in {} ({} messages)",
        id.as_str(),
        read.header.cwd.display(),
        messages.len()
    )];
    if read.truncated_tail {
        notices.push(
            "the session file had a truncated last line, so rho dropped it. A crash can leave \
             one."
                .to_string(),
        );
    }
    if read.dropped_records > 0 {
        notices.push(format!(
            "{} records in the session file did not decode, so rho skipped them.",
            read.dropped_records
        ));
    }

    let mut writer = store.append_to(&path)?;
    // A resume under another model states the change on disk, so a reader of the file can see
    // which model answered which turn.
    let stored_model = stored_model(&read);
    if stored_model.as_deref() != Some(request.model) {
        writer.append(
            Record::ModelChange {
                provider: request.provider.to_string(),
                model: request.model.to_string(),
            },
            None,
        )?;
    }
    Ok(Recording {
        recorder: SessionRecorder::new(SessionLog::File(writer)),
        messages,
        notices,
        id: Some(id),
        path: Some(path),
        _lock: Some(lock),
    })
}

/// The newest model the file states.
fn stored_model(read: &rho_core::ReadResult) -> Option<String> {
    read.entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.record {
            Record::ModelChange { model, .. } => Some(model.clone()),
            _ => None,
        })
}

/// The file one session id lives in.
fn store_path(store: &SessionStore, id: &SessionId) -> PathBuf {
    store
        .rows()
        .ok()
        .and_then(|rows| {
            rows.into_iter().find_map(|row| match row {
                rho_core::SessionRow::Session(summary) if summary.id == *id => Some(summary.path),
                _ => None,
            })
        })
        .unwrap_or_else(|| store.path_of(id))
}

/// Which session the selector names.
fn target(
    store: &SessionStore,
    key: &ProjectKey,
    selector: &SessionSelector,
) -> anyhow::Result<SessionId> {
    match selector {
        SessionSelector::New => unreachable!("a new session never resolves a target"),
        SessionSelector::Newest => match store.newest_open()? {
            Some(id) => Ok(id),
            None => Err(SessionError::NoSessionToContinue {
                project: key.as_str().to_string(),
            }
            .into()),
        },
        SessionSelector::Named(prefix) => match store.resolve_prefix(prefix)? {
            PrefixMatch::One(id) => Ok(id),
            PrefixMatch::None => Err(SessionError::NoSuchSession {
                prefix: prefix.clone(),
            }
            .into()),
            PrefixMatch::Many(matches) => Err(SessionError::AmbiguousPrefix {
                prefix: prefix.clone(),
                matches: matches.iter().map(|id| id.as_str().to_string()).collect(),
            }
            .into()),
        },
    }
}
