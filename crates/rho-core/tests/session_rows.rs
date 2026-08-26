//! The row builder, the prefix resolver, the newest open session, and the session lock.
//!
//! See `SPEC-session-store-wiring` sections 5, 5a, 7, 7d, and 8e, and the decisions
//! `D-a-bad-session-file-is-one-row`, `D-a-session-title-costs-nothing`,
//! `D-a-live-session-holds-a-lock`, and `D-the-budget-test-needs-an-observable-difference`.
//!
//! Every test isolates the filesystem with `tempfile`. None sleeps, none reads the real home
//! directory, and none touches the network.

use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rho_core::{
    ContentBlock, Message, NewSession, PrefixMatch, ROW_HEAD_LINES, ROW_TAIL_BYTES, Record, Role,
    RowMeta, SessionError, SessionId, SessionRow, SessionStore, Usage, row_from,
};

/// A source that counts every byte it hands out.
///
/// It sits **under** any buffered reader, so a full forward scan is counted rather than
/// hidden by a buffer. It hands out a small chunk per `fill_buf`, exactly as a real
/// `BufReader` over a file does, so no implementation can borrow the whole file uncounted.
struct CountingSource {
    data: Vec<u8>,
    position: usize,
    /// The end of the chunk currently lent out, so `consume` never counts twice.
    lent_to: usize,
    counted: Arc<AtomicUsize>,
    chunk: usize,
}

impl CountingSource {
    fn new(data: Vec<u8>, chunk: usize) -> (Self, Arc<AtomicUsize>) {
        let counted = Arc::new(AtomicUsize::new(0));
        (
            Self {
                data,
                position: 0,
                lent_to: 0,
                counted: Arc::clone(&counted),
                chunk,
            },
            counted,
        )
    }
}

impl Read for CountingSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.data.len().saturating_sub(self.position);
        let take = available.min(buf.len()).min(self.chunk);
        buf[..take].copy_from_slice(&self.data[self.position..self.position + take]);
        self.position += take;
        self.counted.fetch_add(take, Ordering::Relaxed);
        Ok(take)
    }
}

impl BufRead for CountingSource {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let end = (self.position + self.chunk).min(self.data.len());
        // Count each byte once, the first time it is lent out.
        if end > self.lent_to {
            self.counted
                .fetch_add(end - self.lent_to, Ordering::Relaxed);
            self.lent_to = end;
        }
        Ok(&self.data[self.position..end])
    }

    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.data.len());
    }
}

impl Seek for CountingSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let length = self.data.len() as i64;
        let target = match from {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => length + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };
        let target = target.clamp(0, length) as usize;
        self.position = target;
        // A seek discards the lent chunk, so the next `fill_buf` counts again from here.
        self.lent_to = target;
        Ok(target as u64)
    }
}

/// A store under a temporary directory, and the root it uses.
fn temp_store() -> (tempfile::TempDir, SessionStore, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path().join("sessions");
    let store = SessionStore::new(&root);
    (dir, store, root)
}

fn id(suffix: u16) -> SessionId {
    SessionId::mint(1_756_000_000_000, suffix)
}

/// An id minted at a given second, so a test can order two sessions with no sleep.
fn id_at(second: u64, suffix: u16) -> SessionId {
    SessionId::mint(1_756_000_000_000 + second * 1000, suffix)
}

fn new_session<'a>(id: &'a SessionId, cwd: &'a Path) -> NewSession<'a> {
    NewSession {
        id,
        cwd,
        approval: "read-only",
        sandbox: "off",
        provider: "testkit",
        model: "test-model",
        forked_from: None,
    }
}

fn user_message(text: &str) -> Record {
    Record::Message {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        },
    }
}

/// A session file with a first prompt, written through the store.
fn session_with_prompt(store: &SessionStore, id: &SessionId, prompt: &str) -> PathBuf {
    let mut writer = store
        .create(new_session(id, Path::new("/work")))
        .expect("a created session");
    let head = writer.head();
    writer.append(user_message(prompt), head).expect("appended");
    writer.path().to_path_buf()
}

// ---------------------------------------------------------------------------
// Section 5a. The row builder reads a bounded head and a bounded tail.
// ---------------------------------------------------------------------------

#[test]
fn a_row_never_decodes_the_whole_file() {
    // The bound needs a seam, or the test is theatre. A full-file implementation produces the
    // same finished row, so only a counting source can tell the two apart.
    //
    // The `display_path` does not exist, so an implementation that opens a path instead of
    // reading the source fails as well.
    let mut file = String::new();
    file.push_str(&header_line(id(0x0001).as_str(), 1_756_000_000_000));
    file.push('\n');
    file.push_str(&model_line("r1", "r0"));
    file.push('\n');
    file.push_str(&message_line("r2", "r1", "the first prompt"));
    file.push('\n');
    // Half a megabyte of middle, far past the head window and the tail window together.
    for n in 0..2000 {
        file.push_str(&message_line(
            &format!("r{}", n + 3),
            &format!("r{}", n + 2),
            &format!("filler {n} {}", "x".repeat(200)),
        ));
        file.push('\n');
    }
    let size = file.len() as u64;
    assert!(
        size > ROW_TAIL_BYTES * 4,
        "the fixture must be far larger than the tail window, got {size}"
    );
    let (source, counted) = CountingSource::new(file.into_bytes(), 4096);

    let row = row_from(
        source,
        RowMeta {
            display_path: PathBuf::from("/nonexistent/never-opened.jsonl"),
            size_bytes: size,
            last_active_millis: 1_756_000_009_000,
        },
    );

    let SessionRow::Session(summary) = row else {
        panic!("a readable file must build a row");
    };
    assert_eq!(summary.id.as_str(), id(0x0001).as_str());
    let bytes = counted.load(Ordering::Relaxed) as u64;
    // The head is at most `ROW_HEAD_LINES` lines, the tail is at most `ROW_TAIL_BYTES`, and a
    // buffered reader may hold one more chunk of each.
    let allowed = ROW_TAIL_BYTES + (ROW_HEAD_LINES as u64 * 4096) + 2 * 4096;
    assert!(
        bytes <= allowed,
        "a row must read at most {allowed} bytes of a {size} byte file, it read {bytes}"
    );
}

#[test]
fn rows_read_only_the_head_and_the_tail() {
    // The ported list test. It keeps the assertions on the path, the working directory, and
    // the size, and it reads the id as a `SessionId`.
    let (_guard, store, _root) = temp_store();
    for n in 0..5 {
        session_with_prompt(&store, &id_at(n, 1), "body");
    }

    let rows = store.rows().expect("the rows build");

    assert_eq!(rows.len(), 5, "rows finds every session file");
    for row in &rows {
        let SessionRow::Session(summary) = row else {
            panic!("every file here is readable");
        };
        assert_eq!(
            summary.cwd,
            Path::new("/work"),
            "the row states the directory from the header"
        );
        assert!(summary.size_bytes > 0, "the row carries file metadata");
        assert!(
            summary.path.exists(),
            "the row path points at a real session file, not an empty default"
        );
        SessionId::parse(summary.id.as_str()).expect("the row id is a session id");
    }
}

#[test]
fn a_row_reports_the_cumulative_usage() {
    let (_guard, store, _root) = temp_store();
    let usage = Usage {
        input_tokens: 1200,
        output_tokens: 340,
        cache_read_tokens: 10,
        cache_write_tokens: 5,
        cost_usd: Some(0.0821),
    };
    {
        let mut writer = store
            .create(new_session(&id(0x0002), Path::new("/work")))
            .expect("a created session");
        let head = writer.head();
        writer
            .append(user_message("a prompt"), head)
            .expect("append");
        // Two usage records. The newest one is cumulative, so the row must take the last.
        writer
            .append(
                Record::Usage {
                    usage: Usage {
                        input_tokens: 100,
                        ..usage
                    },
                },
                None,
            )
            .expect("append");
        writer
            .append(Record::Usage { usage }, None)
            .expect("append");
    }

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(
        summary.usage,
        Some(usage),
        "the row reports the last cumulative usage record"
    );
}

#[test]
fn a_row_reports_no_usage_for_a_session_with_no_turn() {
    let (_guard, store, _root) = temp_store();
    session_with_prompt(&store, &id(0x0003), "a prompt with no answer");

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(
        summary.usage, None,
        "rho shows no number it did not read: a session with no turn reports no usage"
    );
}

#[test]
fn a_row_states_the_model_from_the_second_line() {
    let (_guard, store, _root) = temp_store();
    session_with_prompt(&store, &id(0x0004), "a prompt");

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(
        summary.model, "test-model",
        "`create` writes the model record, so the row reads it from the head"
    );
}

#[test]
fn a_row_states_its_start_time_and_its_last_activity() {
    let (_guard, store, _root) = temp_store();
    let path = session_with_prompt(&store, &id(0x0005), "a prompt");
    let modified = std::fs::metadata(&path)
        .expect("the file")
        .modified()
        .expect("a modification time")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_millis() as u64;

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert!(
        summary.started_millis > 0,
        "the start time comes from the header timestamp"
    );
    // Within one second, because a filesystem may hold a coarser modification time.
    assert!(
        summary.last_active_millis.abs_diff(modified) <= 1000,
        "the last activity comes from the file metadata, got {} against {modified}",
        summary.last_active_millis
    );
}

#[test]
fn a_row_states_its_size() {
    let (_guard, store, _root) = temp_store();
    let path = session_with_prompt(&store, &id(0x0006), "a prompt");
    let length = std::fs::metadata(&path).expect("the file").len();

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(summary.size_bytes, length, "the row states the file length");
}

#[test]
fn a_row_prefers_an_explicit_name() {
    let (_guard, store, _root) = temp_store();
    {
        let mut writer = store
            .create(new_session(&id(0x0007), Path::new("/work")))
            .expect("a created session");
        let head = writer.head();
        writer
            .append(user_message("the first prompt"), head)
            .expect("append");
        writer
            .append(
                Record::Name {
                    title: "an explicit name".to_string(),
                },
                None,
            )
            .expect("append");
    }

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(summary.title, "an explicit name");
    assert!(summary.title_is_explicit, "a Name record set the title");
}

#[test]
fn the_newest_name_record_wins() {
    let (_guard, store, _root) = temp_store();
    {
        let mut writer = store
            .create(new_session(&id(0x0008), Path::new("/work")))
            .expect("a created session");
        let head = writer.head();
        writer
            .append(user_message("a prompt"), head)
            .expect("append");
        for title in ["the first name", "the second name"] {
            writer
                .append(
                    Record::Name {
                        title: title.to_string(),
                    },
                    None,
                )
                .expect("append");
        }
    }

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(
        summary.title, "the second name",
        "the newest Name record wins"
    );
}

#[test]
fn a_row_falls_back_to_the_first_prompt() {
    // Two prompts, because one cannot prove both rules. A long first line would fill the cap
    // and hide a missing one-line rule, and a short first line cannot show the cap.
    let (_guard, store, _root) = temp_store();
    let short_first_line = id_at(0, 0x0009);
    let long_first_line = id_at(60, 0x000a);
    session_with_prompt(
        &store,
        &short_first_line,
        "fix the parser\nand a second line nobody wants in a list",
    );
    session_with_prompt(&store, &long_first_line, &"s".repeat(90));

    let rows = store.rows().expect("the rows build");
    let row = |wanted: &SessionId| {
        rows.iter()
            .find_map(|row| match row {
                SessionRow::Session(summary) if summary.id == *wanted => Some(summary.clone()),
                _ => None,
            })
            .expect("the row")
    };

    let short = row(&short_first_line);
    assert!(
        !short.title_is_explicit,
        "no Name record, so the title is not explicit"
    );
    assert_eq!(
        short.title, "fix the parser",
        "the title is the first line of the prompt, and never the whole prompt"
    );

    let long = row(&long_first_line);
    assert_eq!(
        long.title.len(),
        60,
        "a long first line is cut at 60 bytes, got {:?}",
        long.title
    );
}

#[test]
fn a_new_session_titles_itself_from_the_first_prompt() {
    // The same rule, stated as the feature it is. A session needs no explicit name to have a
    // readable title, and the title costs no model call.
    let (_guard, store, _root) = temp_store();
    session_with_prompt(&store, &id(0x000a), "fix the parser");

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };

    assert_eq!(summary.title, "fix the parser");
}

#[test]
fn a_tail_read_drops_a_partial_first_line() {
    // A tail that starts inside a line must yield no broken record. The `Name` record here
    // sits at the very end, so the tail read must recover after the partial line it lands in.
    let mut file = String::new();
    file.push_str(&header_line(id(0x000b).as_str(), 1_756_000_000_000));
    file.push('\n');
    file.push_str(&model_line("r1", "r0"));
    file.push('\n');
    file.push_str(&message_line("r2", "r1", "the first prompt"));
    file.push('\n');
    // Enough filler that the tail window starts in the middle of a line.
    let mut n = 0;
    while file.len() < (ROW_TAIL_BYTES as usize) * 2 {
        file.push_str(&message_line(
            &format!("r{}", n + 3),
            &format!("r{}", n + 2),
            &format!("filler {n} {}", "y".repeat(300)),
        ));
        file.push('\n');
        n += 1;
    }
    file.push_str(&name_line("rlast", "the name at the end"));
    file.push('\n');
    let size = file.len() as u64;
    let (source, _counted) = CountingSource::new(file.into_bytes(), 4096);

    let row = row_from(
        source,
        RowMeta {
            display_path: PathBuf::from("/nonexistent/never-opened.jsonl"),
            size_bytes: size,
            last_active_millis: 1_756_000_009_000,
        },
    );

    let SessionRow::Session(summary) = row else {
        panic!("a partial first line in the tail must not break the row");
    };
    assert_eq!(summary.title, "the name at the end");
    assert!(summary.title_is_explicit);
}

#[test]
fn a_row_marks_a_closed_session() {
    let (_guard, store, _root) = temp_store();
    {
        let mut writer = store
            .create(new_session(&id(0x000c), Path::new("/work")))
            .expect("a created session");
        let head = writer.head();
        writer
            .append(user_message("a prompt"), head)
            .expect("append");
        writer.close().expect("closed");
    }
    session_with_prompt(&store, &id(0x000d), "an open session");

    let rows = store.rows().expect("the rows build");
    let closed: Vec<bool> = rows
        .iter()
        .map(|row| match row {
            SessionRow::Session(summary) => summary.closed,
            SessionRow::Unreadable { .. } => panic!("both files are readable"),
        })
        .collect();

    assert_eq!(
        closed.iter().filter(|c| **c).count(),
        1,
        "exactly one of the two sessions is closed"
    );
}

#[test]
fn a_torn_last_line_does_not_produce_a_wrong_closed_flag() {
    // An append with no fsync can leave half a line. Half of a `Closed` record must never read
    // as a close.
    let (_guard, store, _root) = temp_store();
    let path = session_with_prompt(&store, &id(0x000e), "a prompt");
    let mut text = std::fs::read_to_string(&path).expect("the file");
    text.push_str(
        "{\"id\":\"r9\",\"parentId\":\"r2\",\"timestamp\":\"1756000009000\",\"type\":\"clo",
    );
    std::fs::write(&path, text).expect("the torn file");

    let rows = store.rows().expect("the rows build");
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a torn last line is not an unreadable file");
    };

    assert!(!summary.closed, "half a Closed record is not a close");
}

#[test]
fn one_unreadable_file_is_one_row() {
    // The invariant: for any directory of N session files, `rows` returns N rows. One
    // unreadable file must never fail the whole list.
    for good in [0usize, 1, 3] {
        let (_guard, store, root) = temp_store();
        for n in 0..good {
            session_with_prompt(&store, &id_at(n as u64, 1), "body");
        }
        std::fs::create_dir_all(&root).expect("the directory");
        let broken = root.join("20260101-000000-dead.jsonl");
        std::fs::write(&broken, "this is not a session header\n").expect("the broken file");
        let empty = root.join("20260101-000001-beef.jsonl");
        std::fs::write(&empty, "").expect("the empty file");
        // A file rho cannot even open. Without this case the list could propagate an open
        // error and no test would see it, because a file with bad content still opens.
        let denied = root.join("20260101-000002-0bad.jsonl");
        std::fs::write(&denied, "{}\n").expect("the denied file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000))
                .expect("the mode");
        }

        let rows = store.rows().expect("one bad file must not fail the list");

        assert_eq!(
            rows.len(),
            good + 3,
            "a directory of {} files must give {} rows",
            good + 3,
            good + 3
        );
        let unreadable: Vec<&SessionRow> = rows
            .iter()
            .filter(|row| matches!(row, SessionRow::Unreadable { .. }))
            .collect();
        assert_eq!(unreadable.len(), 3, "every bad file is one row");
        for row in unreadable {
            let SessionRow::Unreadable { path, reason } = row else {
                unreachable!();
            };
            assert!(path.exists(), "the row names the real file");
            assert!(!reason.is_empty(), "the row states a reason");
        }
    }
}

#[test]
fn an_empty_file_is_an_unreadable_row() {
    let (_guard, store, root) = temp_store();
    std::fs::create_dir_all(&root).expect("the directory");
    std::fs::write(root.join("20260101-000000-0000.jsonl"), "").expect("the empty file");

    let rows = store.rows().expect("an empty file must not panic");

    assert!(matches!(rows[0], SessionRow::Unreadable { .. }));
}

#[test]
fn a_file_smaller_than_the_tail_window_builds_a_row() {
    // A two line file needs no seek at all, and the tail window is larger than the file.
    let mut file = String::new();
    file.push_str(&header_line(id(0x000f).as_str(), 1_756_000_000_000));
    file.push('\n');
    file.push_str(&model_line("r1", "r0"));
    file.push('\n');
    let size = file.len() as u64;
    assert!(size < ROW_TAIL_BYTES);
    let (source, _counted) = CountingSource::new(file.into_bytes(), 4096);

    let row = row_from(
        source,
        RowMeta {
            display_path: PathBuf::from("/nonexistent/never-opened.jsonl"),
            size_bytes: size,
            last_active_millis: 1_756_000_009_000,
        },
    );

    let SessionRow::Session(summary) = row else {
        panic!("a short file must build a row");
    };
    assert_eq!(summary.title, "", "no prompt yet, so no title");
    assert_eq!(summary.model, "test-model");
}

#[test]
fn a_first_prompt_beyond_the_head_lines_leaves_the_title_empty() {
    // The row states no title rather than a wrong one. A bounded read cannot see a prompt
    // that sits past the head window.
    let mut file = String::new();
    file.push_str(&header_line(id(0x0010).as_str(), 1_756_000_000_000));
    file.push('\n');
    file.push_str(&model_line("r1", "r0"));
    file.push('\n');
    for n in 0..(ROW_HEAD_LINES + 4) {
        file.push_str(&usage_line(&format!("r{}", n + 2), &format!("r{}", n + 1)));
        file.push('\n');
    }
    file.push_str(&message_line("rp", "r2", "a prompt past the head window"));
    file.push('\n');
    // The tail window would see it, so the file must be larger than the tail window.
    let mut n = 0;
    while file.len() < (ROW_TAIL_BYTES as usize) * 2 {
        file.push_str(&usage_line(&format!("rf{n}"), "r2"));
        file.push('\n');
        n += 1;
    }
    let size = file.len() as u64;
    let (source, _counted) = CountingSource::new(file.into_bytes(), 4096);

    let row = row_from(
        source,
        RowMeta {
            display_path: PathBuf::from("/nonexistent/never-opened.jsonl"),
            size_bytes: size,
            last_active_millis: 1_756_000_009_000,
        },
    );

    let SessionRow::Session(summary) = row else {
        panic!("a readable row");
    };
    assert_eq!(
        summary.title, "",
        "the row states no title rather than a wrong one"
    );
}

#[test]
fn rows_come_back_newest_first() {
    let (_guard, store, _root) = temp_store();
    let older = id_at(0, 0x0001);
    let newer = id_at(60, 0x0002);
    session_with_prompt(&store, &older, "the older session");
    session_with_prompt(&store, &newer, "the newer session");

    let rows = store.rows().expect("the rows build");
    let ids: Vec<String> = rows
        .iter()
        .map(|row| match row {
            SessionRow::Session(summary) => summary.id.as_str().to_string(),
            SessionRow::Unreadable { path, .. } => path.display().to_string(),
        })
        .collect();

    assert_eq!(
        ids,
        vec![newer.as_str().to_string(), older.as_str().to_string()],
        "the order follows the id, newest first"
    );
}

#[test]
fn a_row_shows_its_fork_origin() {
    let (_guard, store, _root) = temp_store();
    let source = id(0x0011);
    let path = session_with_prompt(&store, &source, "the original");
    let head = {
        let read = rho_core::SessionReader::read(&path).expect("the file reads back");
        read.entries.last().expect("a record").id.clone()
    };
    let forked = id(0x0012);
    store.fork(&path, &head, &forked).expect("a fork");

    let rows = store.rows().expect("the rows build");
    let row = rows
        .iter()
        .find_map(|row| match row {
            SessionRow::Session(summary) if summary.id == forked => Some(summary),
            _ => None,
        })
        .expect("the forked row");

    let origin = row.forked_from.as_ref().expect("a fork origin");
    assert_eq!(origin.session_id, source.as_str());
    assert_eq!(origin.record_id, head);
}

// ---------------------------------------------------------------------------
// Section 7. The prefix, and the newest open session.
// ---------------------------------------------------------------------------

#[test]
fn a_unique_prefix_resolves_to_one_session() {
    let (_guard, store, _root) = temp_store();
    let wanted = id_at(0, 0xabcd);
    session_with_prompt(&store, &wanted, "one");
    session_with_prompt(&store, &id_at(3600, 0x0002), "two");

    let resolved = store
        .resolve_prefix(&wanted.as_str()[..13])
        .expect("the prefix resolves");

    assert_eq!(resolved, PrefixMatch::One(wanted));
}

#[test]
fn an_ambiguous_prefix_lists_every_match() {
    let (_guard, store, _root) = temp_store();
    let first = id_at(0, 0x0001);
    let second = id_at(0, 0x0002);
    session_with_prompt(&store, &first, "one");
    session_with_prompt(&store, &second, "two");
    // The two ids share everything but the suffix.
    let shared = &first.as_str()[..15];

    let resolved = store.resolve_prefix(shared).expect("the prefix resolves");

    let PrefixMatch::Many(matches) = resolved else {
        panic!("a shared prefix must return every match, got {resolved:?}");
    };
    assert_eq!(matches.len(), 2);
    assert!(matches.contains(&first) && matches.contains(&second));

    // The error a command builds from it must name every match.
    let error = SessionError::AmbiguousPrefix {
        prefix: shared.to_string(),
        matches: matches.iter().map(|id| id.as_str().to_string()).collect(),
    }
    .to_string();
    assert!(error.contains(first.as_str()), "the message lists {error}");
    assert!(error.contains(second.as_str()), "the message lists {error}");
}

#[test]
fn an_unknown_prefix_resolves_to_none() {
    let (_guard, store, _root) = temp_store();
    session_with_prompt(&store, &id(0x0013), "one");

    let resolved = store.resolve_prefix("19700101-00").expect("no error");

    assert_eq!(resolved, PrefixMatch::None);
}

#[test]
fn a_crash_offers_the_unclosed_session() {
    let (_guard, store, _root) = temp_store();
    let closed = id_at(0, 0x0001);
    let open = id_at(60, 0x0002);
    {
        let mut writer = store
            .create(new_session(&closed, Path::new("/work")))
            .expect("a created session");
        writer.close().expect("closed");
    }
    session_with_prompt(&store, &open, "a session that never closed");

    let offered = store.newest_open().expect("no error");

    assert_eq!(
        offered,
        Some(open),
        "a crash offers the newest session with no close record"
    );
}

#[test]
fn a_closed_session_is_never_offered() {
    let (_guard, store, _root) = temp_store();
    for n in 0..3 {
        let mut writer = store
            .create(new_session(&id_at(n, 1), Path::new("/work")))
            .expect("a created session");
        writer.close().expect("closed");
    }

    let offered = store.newest_open().expect("no error");

    assert_eq!(offered, None, "a store of closed sessions offers nothing");
}

// ---------------------------------------------------------------------------
// Section 7d. A live session holds an advisory lock.
// ---------------------------------------------------------------------------

#[test]
fn a_second_process_cannot_open_a_live_session() {
    // Two worktrees share one project key, so `--continue` in both can open one file. Both
    // would seed their ids from the same read, and the lines would interleave.
    //
    // The second lock runs in a child process, because `flock` is held per open file and a
    // second lock inside one process may be granted by the operating system.
    let (_guard, store, root) = temp_store();
    let session = id(0x0014);
    session_with_prompt(&store, &session, "a live session");

    let held = store.lock(&session).expect("the first lock");

    let busy = lock_in_a_child(&root, &session);
    assert_eq!(
        busy, "busy",
        "a second process must be refused while the first holds the lock"
    );
    let message = SessionError::Busy {
        id: session.as_str().to_string(),
    }
    .to_string();
    assert!(
        message.contains(session.as_str()),
        "the refusal must name the session, got {message}"
    );
    drop(held);
}

#[test]
fn a_lock_is_released_when_the_process_ends() {
    // Dropping the lock frees the session. The operating system does the same when a process
    // dies, so a crash never leaves a session locked for ever.
    let (_guard, store, root) = temp_store();
    let session = id(0x0015);
    session_with_prompt(&store, &session, "a session");

    {
        let _held = store.lock(&session).expect("the first lock");
        assert_eq!(lock_in_a_child(&root, &session), "busy");
    }

    assert_eq!(
        lock_in_a_child(&root, &session),
        "locked",
        "a dropped lock frees the session"
    );
}

#[test]
fn newest_open_skips_a_locked_session() {
    let (_guard, store, _root) = temp_store();
    let live = id_at(60, 0x0001);
    let free = id_at(0, 0x0002);
    session_with_prompt(&store, &live, "the live session");
    session_with_prompt(&store, &free, "the free session");

    let _held = store.lock(&live).expect("the lock");
    let offered = store.newest_open().expect("no error");

    assert_eq!(
        offered,
        Some(free),
        "--continue must move past a live session and take the next one"
    );
}

#[test]
fn a_read_only_command_needs_no_lock() {
    let (_guard, store, _root) = temp_store();
    let session = id(0x0016);
    session_with_prompt(&store, &session, "a locked session");
    let _held = store.lock(&session).expect("the lock");

    let rows = store.rows().expect("a list must work on a locked session");
    let resolved = store
        .resolve_prefix(session.as_str())
        .expect("a resolve must work on a locked session");

    assert_eq!(rows.len(), 1);
    assert_eq!(resolved, PrefixMatch::One(session));
}

#[test]
fn a_filesystem_that_cannot_lock_is_refused() {
    // A store root that is not a directory cannot hold a lock file. A network filesystem that
    // silently ignores a lock would fail open, and that is the shape of
    // `D-plugin-does-not-classify-itself`. So the refusal is the rule.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let blocked = dir.path().join("not-a-directory");
    std::fs::write(&blocked, "a file where a directory must be").expect("the file");
    let store = SessionStore::new(&blocked);

    let error = store
        .lock(&id(0x0017))
        .map(|_| ())
        .expect_err("a filesystem that cannot lock must be refused");

    assert!(
        matches!(error, SessionError::LockUnsupported { .. }),
        "expected LockUnsupported, got {error:?}"
    );
}

#[test]
fn a_lock_file_that_cannot_open_is_refused() {
    // The store directory exists, and the lock path is a directory, so the open fails. Without
    // this case the only test of the refusal never reached `take_lock` at all, because the
    // store root check refused first.
    let (_guard, store, root) = temp_store();
    let session = id(0x0018);
    session_with_prompt(&store, &session, "a session");
    std::fs::create_dir(root.join(format!("{}.lock", session.as_str()))).expect("the directory");

    let error = store
        .lock(&session)
        .map(|_| ())
        .expect_err("a lock file that cannot open must be refused");

    assert!(
        matches!(error, SessionError::LockUnsupported { .. }),
        "expected LockUnsupported, got {error:?}"
    );
}

#[test]
fn only_a_would_block_error_means_busy() {
    // The classification is pure, because a filesystem that refuses to lock cannot be arranged
    // on a developer machine. This is the fail-open risk: a warning that continued would be the
    // shape of `D-plugin-does-not-classify-itself`.
    let path = Path::new("/tmp/never-opened.lock");

    for code in [libc::EWOULDBLOCK, libc::EAGAIN] {
        let error = rho_core::classify_lock_failure(Some(code), path, "20260101-000000-0000");
        assert!(
            matches!(error, SessionError::Busy { .. }),
            "code {code} means another process holds the lock, got {error:?}"
        );
    }
    // Every other code, and an unknown one, is a refusal.
    for code in [
        Some(libc::ENOLCK),
        Some(libc::EOPNOTSUPP),
        Some(libc::EBADF),
        None,
    ] {
        let error = rho_core::classify_lock_failure(code, path, "20260101-000000-0000");
        assert!(
            matches!(error, SessionError::LockUnsupported { .. }),
            "code {code:?} must refuse the run, got {error:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Section 8e. The budget.
// ---------------------------------------------------------------------------

#[test]
fn a_list_of_five_hundred_sessions_reads_only_the_head_and_the_tail() {
    // The deterministic half. One of the 500 files is a sentinel: it carries a `Name` record
    // after the head window and more than `ROW_TAIL_BYTES` before the end. A bounded `rows`
    // cannot see that record, so the sentinel row falls back to the first prompt. A `rows`
    // that decodes the whole file reports the explicit name instead.
    //
    // See `D-the-budget-test-needs-an-observable-difference`. The first draft asserted a byte
    // bound over a method that opens its own files, so no counting source could see the
    // defect.
    let (_guard, store, root) = temp_store();
    std::fs::create_dir_all(&root).expect("the directory");
    for n in 0..499u64 {
        session_with_prompt(&store, &id_at(n, 0x0001), "an ordinary session");
    }
    let sentinel = id_at(600, 0x0002);
    let mut file = String::new();
    file.push_str(&header_line(sentinel.as_str(), 1_756_000_000_000));
    file.push('\n');
    file.push_str(&model_line("r1", "r0"));
    file.push('\n');
    file.push_str(&message_line("r2", "r1", "the first prompt"));
    file.push('\n');
    // Push the head window past, so the name below is outside it too.
    for n in 0..ROW_HEAD_LINES {
        file.push_str(&usage_line(&format!("rh{n}"), "r2"));
        file.push('\n');
    }
    // The hidden name. It sits past the head window.
    file.push_str(&name_line("r3", "a name only a full decode can see"));
    file.push('\n');
    // Then more than the tail window of filler, so the tail read cannot reach the name.
    let mut n = 0;
    while file.len() < (ROW_TAIL_BYTES as usize) * 2 {
        file.push_str(&usage_line(&format!("rf{n}"), "r2"));
        file.push('\n');
        n += 1;
    }
    std::fs::write(root.join(format!("{}.jsonl", sentinel.as_str())), &file)
        .expect("the sentinel file");

    let started = std::time::Instant::now();
    let rows = store.rows().expect("the rows build");
    let elapsed = started.elapsed();

    assert_eq!(rows.len(), 500, "every file is one row");
    let row = rows
        .iter()
        .find_map(|row| match row {
            SessionRow::Session(summary) if summary.id == sentinel => Some(summary),
            _ => None,
        })
        .expect("the sentinel row");
    assert!(
        !row.title_is_explicit,
        "a bounded read cannot see a Name record past the tail window, so this row must fall \
         back to the first prompt; it reported the explicit title {:?}",
        row.title
    );
    assert_eq!(row.title, "the first prompt");
    // The wall clock is measured and printed, and it asserts nothing. A shared runner makes a
    // 100 millisecond assertion flaky, and a fast machine would pass a full decode of small
    // files. See `D-a-budget-is-measured-not-asserted`.
    println!("500 rows in {elapsed:?}");
}

// ---------------------------------------------------------------------------
// The hand-built lines these tests need.
// ---------------------------------------------------------------------------

fn header_line(session_id: &str, millis: u64) -> String {
    format!(
        r#"{{"id":"r0","parentId":null,"timestamp":"{millis}","type":"session","version":1,"cwd":"/work","approval":"read-only","sandbox":"off","session_id":"{session_id}"}}"#
    )
}

fn model_line(record_id: &str, parent: &str) -> String {
    format!(
        r#"{{"id":"{record_id}","parentId":"{parent}","timestamp":"1756000000001","type":"model_change","provider":"testkit","model":"test-model"}}"#
    )
}

fn message_line(record_id: &str, parent: &str, text: &str) -> String {
    format!(
        r#"{{"id":"{record_id}","parentId":"{parent}","timestamp":"1756000000002","type":"message","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

fn name_line(record_id: &str, title: &str) -> String {
    format!(
        r#"{{"id":"{record_id}","parentId":null,"timestamp":"1756000000003","type":"name","title":"{title}"}}"#
    )
}

fn usage_line(record_id: &str, parent: &str) -> String {
    format!(
        r#"{{"id":"{record_id}","parentId":"{parent}","timestamp":"1756000000004","type":"usage","usage":{{"input_tokens":1,"output_tokens":1,"cache_read_tokens":0,"cache_write_tokens":0,"cost_usd":null}}}}"#
    )
}

/// Take the lock in a child process, and report what happened.
///
/// `flock` is held per open file. A second lock inside one process can be granted, so a test
/// that locked twice in one process would prove nothing about two worktrees. The child runs
/// the real `SessionStore::lock`, through the test binary itself.
fn lock_in_a_child(root: &Path, session: &SessionId) -> String {
    let exe = std::env::current_exe().expect("the test binary");
    let output = std::process::Command::new(exe)
        .arg("--exact")
        .arg("the_child_lock_helper")
        .arg("--nocapture")
        .env("RHO_TEST_LOCK_ROOT", root)
        .env("RHO_TEST_LOCK_ID", session.as_str())
        .output()
        .expect("the child runs");
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("CHILD:busy") {
        "busy".to_string()
    } else if text.contains("CHILD:locked") {
        "locked".to_string()
    } else {
        format!("unexpected child output: {text}")
    }
}

/// The child half of `lock_in_a_child`. It is a test so the harness can run it by name.
///
/// Without the two variables it does nothing, so an ordinary run of the suite is unaffected.
#[test]
fn the_child_lock_helper() {
    let (Ok(root), Ok(id)) = (
        std::env::var("RHO_TEST_LOCK_ROOT"),
        std::env::var("RHO_TEST_LOCK_ID"),
    ) else {
        return;
    };
    let store = SessionStore::new(&root);
    let session = SessionId::parse(&id).expect("a session id");
    match store.lock(&session) {
        Ok(_held) => println!("CHILD:locked"),
        Err(SessionError::Busy { .. }) => println!("CHILD:busy"),
        Err(other) => println!("CHILD:error {other}"),
    }
}
