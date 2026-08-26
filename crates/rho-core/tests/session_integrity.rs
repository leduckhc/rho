//! Referential integrity, id minting, and the mode bits of the session store.
//!
//! See `SPEC-session-store-wiring` sections 6, 6a, 6b, 6c, 7a, 7b, 7c, and 9, and the
//! decisions `D-a-bad-session-file-is-one-row`, `D-a-session-file-is-private`, and
//! `D-a-record-id-is-minted-against-the-set`.
//!
//! Every test here isolates the filesystem with `tempfile`. None reads the real home
//! directory, none sleeps, and none touches the network.

use std::collections::HashSet;
use std::path::Path;

use rho_core::{
    ContentBlock, Entry, ForkOrigin, Message, NewSession, Record, RecordId, Role, SessionError,
    SessionId, SessionReader, SessionStore, branch_messages,
};

/// A store under a temporary directory. The guard must outlive the store.
///
/// The root is returned too, because a test that hand-builds a file needs the path the
/// store will look in. No public method exposes it, and a method only a test wants would be
/// dead surface.
fn temp_store() -> (tempfile::TempDir, SessionStore, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path().join("sessions");
    let store = SessionStore::new(&root);
    (dir, store, root)
}

/// The file a store keeps for one session id.
fn session_path(root: &Path, id: &SessionId) -> std::path::PathBuf {
    root.join(format!("{}.jsonl", id.as_str()))
}

/// A stable session id. Minting takes a time and a suffix, so no test sleeps.
fn id(suffix: u16) -> SessionId {
    SessionId::mint(1_756_000_000_000, suffix)
}

/// The request a test uses to create a session. One struct, per `D-no-four-argument-session-new`.
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

/// The request `create_minted` takes, with no id in it.
fn without_id() -> rho_core::NewSessionWithoutId<'static> {
    rho_core::NewSessionWithoutId {
        cwd: Path::new("/tmp"),
        approval: "read-only",
        sandbox: "off",
        provider: "testkit",
        model: "test-model",
        forked_from: None,
    }
}

/// One user message record.
fn message_record(text: &str) -> Record {
    Record::Message {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        },
    }
}

/// Write the exact lines of a hand-built session file.
fn write_lines(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(path, format!("{}\n", lines.join("\n"))).expect("the file");
}

/// A header line with the two new optional fields left out, as an older rho wrote it.
fn old_header_line() -> String {
    r#"{"id":"r0","parentId":null,"timestamp":"1756000000000","type":"session","version":1,"cwd":"/tmp","approval":"read-only","sandbox":"off"}"#.to_string()
}

/// One message line, hand built, so a test can name any parent it likes.
fn message_line(record_id: &str, parent: Option<&str>, text: &str) -> String {
    let parent = match parent {
        Some(p) => format!("\"{p}\""),
        None => "null".to_string(),
    };
    format!(
        r#"{{"id":"{record_id}","parentId":{parent},"timestamp":"1756000000001","type":"message","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

/// Every record id in one file, in file order.
fn ids_in_file(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("the file");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Section 7e. A record id must print itself.
// ---------------------------------------------------------------------------

#[test]
fn a_record_id_prints_as_its_own_text() {
    let printed = format!("{}", RecordId("r7".to_string()));

    assert_eq!(printed, "r7", "a record id must print as its own text");
}

// ---------------------------------------------------------------------------
// Section 6a. The referential check, run from the child side.
// ---------------------------------------------------------------------------

#[test]
fn an_orphan_refuses_the_file() {
    // `r9` never appears in the file, so `r2` names a parent the reader does not hold.
    let lines = [
        old_header_line(),
        message_line("r2", Some("r9"), "the child of a hole"),
    ];

    let error = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect_err("a record that names a parent the reader does not hold must be refused");

    match error {
        SessionError::Orphan { child, parent } => {
            assert_eq!(child, RecordId("r2".to_string()));
            assert_eq!(parent, RecordId("r9".to_string()));
        }
        other => panic!("expected an orphan error, got {other:?}"),
    }
    // The message must name both ids, because a user has to find the hole.
    let text = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect_err("refused")
        .to_string();
    assert!(
        text.contains("r2"),
        "the message must name the child: {text}"
    );
    assert!(
        text.contains("r9"),
        "the message must name the parent: {text}"
    );
}

#[test]
fn an_unknown_chain_record_is_caught_by_its_children() {
    // A future rho wrote a chain record this build cannot decode. Its child points at it,
    // so the child orphans and the file is refused. This is the test that proves section 6a,
    // and it must fail against a reader that only counts a dropped line.
    let unknown = r#"{"id":"r1","parentId":"r0","timestamp":"1756000000001","type":"rewind_point","target":"r0"}"#;
    let lines = [
        old_header_line(),
        unknown.to_string(),
        message_line("r2", Some("r1"), "after the unknown chain record"),
    ];

    let error = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect_err("an unknown chain record with a child must refuse the file");

    assert!(
        matches!(error, SessionError::Orphan { .. }),
        "expected an orphan, got {error:?}"
    );
}

#[test]
fn an_unknown_leaf_record_is_skipped_and_counted() {
    // The same unknown line, with nothing pointing at it. Nothing points at a leaf, so the
    // file still loads and the drop is counted.
    let unknown = r#"{"id":"r1","parentId":"r0","timestamp":"1756000000001","type":"sticky_note","text":"hello"}"#;
    let lines = [
        old_header_line(),
        unknown.to_string(),
        message_line("r2", Some("r0"), "after the unknown leaf"),
    ];

    let read = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect("an unknown leaf record must not refuse the file");

    assert_eq!(read.entries.len(), 1, "the readable record must survive");
    assert_eq!(read.dropped_records, 1, "the drop must be counted");
}

#[test]
fn a_hand_built_leaf_parent_is_refused() {
    // A `Name` record is a leaf. It is never a parent, so a child that names one is refused.
    // No writer can produce this file, so a test over written files could never fail.
    let name = r#"{"id":"r1","parentId":"r0","timestamp":"1756000000001","type":"name","title":"a title"}"#;
    let lines = [
        old_header_line(),
        name.to_string(),
        message_line("r2", Some("r1"), "the child of a leaf"),
    ];

    let error = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect_err("a leaf record named as a parent must be refused");

    match error {
        SessionError::LeafParent { child, parent } => {
            assert_eq!(child, RecordId("r2".to_string()));
            assert_eq!(parent, RecordId("r1".to_string()));
        }
        other => panic!("expected a leaf-parent error, got {other:?}"),
    }
}

#[test]
fn a_duplicate_id_in_a_file_is_refused() {
    // A hand-edited file with two `r4` records. The refusal runs at read time, before any
    // branch walk, so no caller can walk an ambiguous chain.
    let lines = [
        old_header_line(),
        message_line("r4", Some("r0"), "the first r4"),
        message_line("r4", Some("r0"), "the second r4"),
    ];

    let error = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect_err("two records with one id must refuse the file");

    match error {
        SessionError::DuplicateId { id } => assert_eq!(id, RecordId("r4".to_string())),
        other => panic!("expected a duplicate-id error, got {other:?}"),
    }
}

#[test]
fn an_old_reader_ignores_a_new_header_field() {
    // No type in this module sets `deny_unknown_fields`, so a header from a later rho still
    // parses. A silent drop of a field is fine; a refusal is not.
    let header = r#"{"id":"r0","parentId":null,"timestamp":"1756000000000","type":"session","version":1,"cwd":"/tmp","approval":"read-only","sandbox":"off","retention_days":30}"#;
    let lines = [
        header.to_string(),
        message_line("r1", Some("r0"), "after a header with a new field"),
    ];

    let read = SessionReader::read_from(std::io::Cursor::new(lines.join("\n")))
        .expect("a header with an unknown field must still parse");

    assert_eq!(read.header.version, 1);
    assert_eq!(read.entries.len(), 1);
}

#[test]
fn the_chain_record_set_is_frozen_and_the_version_gates_it() {
    // The runtime half. A file naming a higher version is refused, so a later chain record
    // can never reach this build's walker.
    let header = r#"{"id":"r0","parentId":null,"timestamp":"1756000000000","type":"session","version":2,"cwd":"/tmp","approval":"read-only","sandbox":"off"}"#;

    let error = SessionReader::read_from(std::io::Cursor::new(header))
        .expect_err("a higher version must be refused");

    assert!(
        matches!(error, SessionError::Version(2)),
        "expected a version error, got {error:?}"
    );

    // The compile half. This match has no wildcard, so an eighth chain variant breaks the
    // build and a reader must decide the new variant's class on purpose.
    fn chain_name(record: &Record) -> Option<&'static str> {
        match record {
            Record::Session { .. } => Some("session"),
            Record::ModelChange { .. } => Some("model_change"),
            Record::Message { .. } => Some("message"),
            Record::Usage { .. } => Some("usage"),
            Record::Stop { .. } => Some("stop"),
            Record::Closed => Some("closed"),
            Record::Reopened => Some("reopened"),
            // A leaf. It is never a parent, so it is not in the frozen chain set.
            Record::Name { .. } => None,
        }
    }

    assert_eq!(chain_name(&Record::Closed), Some("closed"));
    assert_eq!(
        chain_name(&Record::Name {
            title: "t".to_string()
        }),
        None,
        "a Name record is a leaf, so it is never a parent"
    );
}

#[test]
fn a_cyclic_parent_chain_is_refused_rather_than_looping() {
    // `check_integrity` checked referential integrity only: every parent resolves, and no parent
    // is a leaf. A cycle satisfies both. So a crafted file whose `r5` names `r6` and whose `r6`
    // names `r5` made every walk loop forever, pushing a clone per iteration.
    //
    // A reviewer found it. This project already guards the same class for subagents, in
    // `check_no_cycle`, "to stop an infinite loop inside a lock". The session reader forgot it.
    //
    // The test has a timeout, because the defect is non-termination and a plain assertion would
    // hang the suite instead of failing it.
    let lines = [
        old_header_line(),
        message_line("r5", Some("r6"), "the first half of a cycle"),
        message_line("r6", Some("r5"), "the second half of a cycle"),
    ];
    let text = lines.join("\n");

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = SessionReader::read_from(std::io::Cursor::new(text));
        let _ = sender.send(match outcome {
            Ok(_) => "accepted".to_string(),
            Err(error) => format!("{error:?}"),
        });
    });
    let settled = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a cyclic file must be refused, and never loop for ever");

    assert!(
        settled.starts_with("CyclicChain"),
        "expected a cyclic-chain refusal, got {settled}"
    );
    assert!(
        SessionError::CyclicChain {
            at: RecordId("r5".to_string())
        }
        .to_string()
        .contains("r5"),
        "the message must name a record on the cycle"
    );
}

#[test]
fn a_hand_built_cyclic_walk_is_refused_rather_than_looping() {
    // Defence in depth. The reader refuses a cyclic file, and a caller that builds entries by hand
    // must be stopped too. One guard on one path is how `confine` stayed unproven.
    let entries = vec![
        Entry {
            id: RecordId("r5".to_string()),
            parent_id: Some(RecordId("r6".to_string())),
            timestamp: "1756000000005".to_string(),
            record: message_record("the first half of a cycle"),
        },
        Entry {
            id: RecordId("r6".to_string()),
            parent_id: Some(RecordId("r5".to_string())),
            timestamp: "1756000000006".to_string(),
            record: message_record("the second half of a cycle"),
        },
    ];

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = branch_messages(&entries, &RecordId("r5".to_string()), None);
        let _ = sender.send(match outcome {
            Ok(messages) => format!("accepted {} messages", messages.len()),
            Err(error) => format!("{error:?}"),
        });
    });
    let settled = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a cyclic walk must be refused, and never loop for ever");

    assert!(
        settled.starts_with("CyclicChain"),
        "expected a cyclic-chain refusal, got {settled}"
    );
}

// ---------------------------------------------------------------------------
// Section 6b. A walk with a hole is an error, never a short list.
// ---------------------------------------------------------------------------

#[test]
fn a_branch_walk_with_a_hole_is_an_error() {
    // Hand-built entries, because `read_from` now refuses this file. A caller that builds
    // entries by hand must still be stopped, so the guard is in the walker too.
    let entries = vec![
        Entry {
            id: RecordId("r1".to_string()),
            parent_id: None,
            timestamp: "1756000000001".to_string(),
            record: message_record("the first"),
        },
        Entry {
            id: RecordId("r3".to_string()),
            parent_id: Some(RecordId("r2".to_string())),
            timestamp: "1756000000003".to_string(),
            record: message_record("the child of a hole"),
        },
    ];

    let error = branch_messages(&entries, &RecordId("r3".to_string()), None)
        .expect_err("a walk that meets a missing parent must be an error");

    match error {
        SessionError::Orphan { child, parent } => {
            assert_eq!(child, RecordId("r3".to_string()));
            assert_eq!(parent, RecordId("r2".to_string()));
        }
        other => panic!("expected an orphan error, got {other:?}"),
    }
}

#[test]
fn a_fork_of_a_chain_with_a_hole_is_an_error() {
    // The same rule on the fork path. Two guards, because one guard on one path is how
    // `confine` stayed unproven.
    let (_guard, store, root) = temp_store();
    let session = id(0x1111);
    let path = {
        let mut writer = store
            .create(new_session(&session, Path::new("/tmp")))
            .expect("a created session");
        writer
            .append(message_record("one"), None)
            .expect("appended");
        writer.path().to_path_buf()
    };
    assert_eq!(path, session_path(&root, &session));
    // Hand-edit the file so the last record names a parent that is not in it.
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .expect("the file")
        .lines()
        .map(str::to_string)
        .collect();
    lines.push(message_line("r9", Some("r7"), "the child of a hole"));
    write_lines(&path, &lines);

    let error = store
        .fork(&path, &RecordId("r9".to_string()), &id(0x2222))
        .map(|writer| writer.path().to_path_buf())
        .expect_err("a fork of a chain with a hole must be an error");

    assert!(
        matches!(error, SessionError::Orphan { .. }),
        "expected the chain to be refused, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// Section 6c. The header states its own id, and the new fields round-trip.
// ---------------------------------------------------------------------------

#[test]
fn a_header_states_its_own_session_id() {
    let (_guard, store, _root) = temp_store();
    let session = id(0x3333);

    let writer = store
        .create(new_session(&session, Path::new("/tmp/project")))
        .expect("a created session");
    // `SessionReader::read` falls back to the file stem when the header states no id, so a
    // test that used it would pass against a header with no id at all. `read_from` has no
    // path, so the id can only come from the header line.
    let text = std::fs::read_to_string(writer.path()).expect("the file");
    let read = SessionReader::read_from(std::io::Cursor::new(&text)).expect("the file reads back");

    assert_eq!(
        read.header.session_id,
        session.as_str(),
        "the header must state the session id, not an empty string"
    );
}

#[test]
fn the_new_header_fields_survive_a_write_and_a_read() {
    let (_guard, store, _root) = temp_store();
    let session = id(0x4444);
    let origin = ForkOrigin {
        session_id: "20260101-000000-abcd".to_string(),
        record_id: RecordId("r5".to_string()),
    };

    let mut request = new_session(&session, Path::new("/tmp/project"));
    request.forked_from = Some(origin.clone());
    let writer = store.create(request).expect("a created session");
    // `read_from`, not `read`, for the same reason as the test above.
    let text = std::fs::read_to_string(writer.path()).expect("the file");
    let read = SessionReader::read_from(std::io::Cursor::new(&text)).expect("the file reads back");

    assert_eq!(read.header.session_id, session.as_str());
    assert_eq!(
        read.header.forked_from,
        Some(origin),
        "the fork origin must come back as it went in"
    );
}

#[test]
fn a_created_file_states_its_model_on_the_second_line() {
    // `create` writes the `ModelChange` record itself, so no caller can forget it and a row
    // reads the model from the head. See section 7.
    let (_guard, store, _root) = temp_store();
    let session = id(0x5555);

    let writer = store
        .create(new_session(&session, Path::new("/tmp")))
        .expect("a created session");
    let text = std::fs::read_to_string(writer.path()).expect("the file");
    let second = text.lines().nth(1).expect("a second line");
    let value: serde_json::Value = serde_json::from_str(second).expect("the second line decodes");

    assert_eq!(value["type"], "model_change");
    assert_eq!(value["provider"], "testkit");
    assert_eq!(value["model"], "test-model");
}

// ---------------------------------------------------------------------------
// Section 7a. A record id is minted against the set, on every store path.
// ---------------------------------------------------------------------------

#[test]
fn two_dropped_records_do_not_mint_a_duplicate_id() {
    // The reader drops records, so a count-based seed mints an id an earlier record holds.
    //
    // The shape matters. Two bad lines sit in the middle, and a good record with a higher id
    // follows them. The reader then holds two entries while the file names ids up to `r4`, so
    // the `entries.len() + 2` seed mints exactly `r4` again.
    //
    // The file is reopened through `SessionStore::append_to`, never seeded by hand, because a
    // test that seeds the writer proves nothing about whether `append_to` remembered to.
    let (_guard, store, root) = temp_store();
    let path = session_path(&root, &id(0x6666));
    write_lines(
        &path,
        &[
            old_header_line(),
            message_line("r1", Some("r0"), "the first message"),
            "{ this is not json".to_string(),
            "{ nor is this".to_string(),
            message_line("r4", Some("r1"), "a good record after two bad"),
        ],
    );
    let before: HashSet<String> = ids_in_file(&path).into_iter().collect();

    let mut writer = store.append_to(&path).expect("the file reopens");
    let minted = writer
        .append(message_record("after"), None)
        .expect("appended");

    assert!(
        !before.contains(&minted.0),
        "a reopened writer must not mint an id the file already holds; it minted {minted:?} \
         over {before:?}"
    );
    let ids = ids_in_file(&path);
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "every id in the file must be unique"
    );
}

#[test]
fn a_fork_of_a_branch_does_not_mint_a_duplicate_id() {
    // The chain is `r1`, `r2`, `r4`, which is not contiguous, so the count-based seed after a
    // fork mints `r4` a second time. `r3` is a sibling and it is not copied.
    let (_guard, store, _root) = temp_store();
    let (path, head) = {
        let mut writer = store
            .create(new_session(&id(0x7777), Path::new("/tmp")))
            .expect("a created session");
        // `create` writes the header as `r0` and the model change as `r1`.
        let base = writer.head().expect("the model change record");
        let r2 = writer
            .append(message_record("one"), Some(base))
            .expect("appended");
        let _r3 = writer
            .append(message_record("a sibling"), Some(r2.clone()))
            .expect("appended");
        let r4 = writer
            .append(message_record("the branch head"), Some(r2))
            .expect("appended");
        (writer.path().to_path_buf(), r4)
    };
    assert_eq!(
        head,
        RecordId("r4".to_string()),
        "the shape this test needs"
    );

    let forked = id(0x8888);
    let mut writer = store.fork(&path, &head, &forked).expect("a fork");
    let before: HashSet<String> = ids_in_file(writer.path()).into_iter().collect();
    let minted = writer
        .append(message_record("after the fork"), None)
        .expect("appended");

    assert!(
        !before.contains(&minted.0),
        "a fork must not mint an id the new file already holds; it minted {minted:?} over \
         {before:?}"
    );
    let ids = ids_in_file(writer.path());
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "every id in the fork must be unique"
    );
}

#[test]
fn a_fork_re_parents_its_first_copied_record() {
    // The first copied record kept its old parent id, which resolved only because every
    // native header happens to be minted as `r0`. An imported file has an eight character
    // hex header id, so the copied record then points at an id the new file does not hold,
    // and the orphan check refuses the fork.
    //
    // So the source here is an imported file, and the assertion never mentions `r0`.
    let (_guard, store, root) = temp_store();
    let path = session_path(&root, &id(0xbbbb));
    let header = r#"{"id":"a1b2c3d4","parentId":null,"timestamp":"1756000000000","type":"session","version":1,"cwd":"/tmp","approval":"read-only","sandbox":"off"}"#;
    write_lines(
        &path,
        &[
            header.to_string(),
            message_line("e5f60718", Some("a1b2c3d4"), "an imported message"),
        ],
    );

    let writer = store
        .fork(&path, &RecordId("e5f60718".to_string()), &id(0xaaaa))
        .expect("a fork of an imported file");
    let new_header_id = ids_in_file(writer.path())
        .first()
        .cloned()
        .expect("a header id");
    let read = SessionReader::read(writer.path()).expect("the fork must read back");
    let first = read.entries.first().expect("one copied record");

    assert_eq!(
        first.parent_id,
        Some(RecordId(new_header_id)),
        "the first copied record must be re-parented onto the new header"
    );
}

#[test]
fn an_imported_file_mints_a_fresh_id() {
    // A pi import gives every record an eight character hex id. So this test pins the
    // invariant and not the example: the minted id is one no record in the file holds.
    let (_guard, store, root) = temp_store();
    let path = session_path(&root, &id(0xbb01));
    let header = r#"{"id":"a1b2c3d4","parentId":null,"timestamp":"1756000000000","type":"session","version":1,"cwd":"/tmp","approval":"read-only","sandbox":"off"}"#;
    let lines = [
        header.to_string(),
        message_line("e5f60718", Some("a1b2c3d4"), "an imported message"),
    ];
    write_lines(&path, &lines);
    let before: HashSet<String> = ids_in_file(&path).into_iter().collect();

    let mut writer = store.append_to(&path).expect("the imported file reopens");
    let minted = writer
        .append(message_record("after"), None)
        .expect("appended");

    assert!(
        !before.contains(&minted.0),
        "an append after an import must mint an id the file does not hold; it minted \
         {minted:?} over {before:?}"
    );
}

#[test]
fn every_record_id_in_a_file_is_unique() {
    // The invariant over a sequence of appends, a fork, and a reopen, all through the store.
    let (_guard, store, _root) = temp_store();
    let first = id(0xcccc);
    let (path, head) = {
        let mut writer = store
            .create(new_session(&first, Path::new("/tmp")))
            .expect("a created session");
        let mut head = writer
            .append(message_record("one"), None)
            .expect("appended");
        for n in 0..5 {
            head = writer
                .append(message_record(&format!("message {n}")), Some(head))
                .expect("appended");
        }
        (writer.path().to_path_buf(), head)
    };
    {
        let mut writer = store.append_to(&path).expect("reopened");
        writer
            .append(message_record("after a reopen"), None)
            .expect("appended");
    }
    {
        let mut writer = store.append_to(&path).expect("reopened twice");
        writer
            .append(message_record("after a second reopen"), None)
            .expect("appended");
    }
    let forked = id(0xdddd);
    let fork_path = {
        let mut writer = store.fork(&path, &head, &forked).expect("a fork");
        writer
            .append(message_record("after the fork"), None)
            .expect("appended");
        writer.path().to_path_buf()
    };

    for file in [&path, &fork_path] {
        let ids = ids_in_file(file);
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "every id in {} must be unique, got {ids:?}",
            file.display()
        );
    }
}

#[test]
fn two_appends_mint_two_ids() {
    // The plain case, so the harder ones above have a baseline.
    let (_guard, store, _root) = temp_store();
    let mut writer = store
        .create(new_session(&id(0xeeee), Path::new("/tmp")))
        .expect("a created session");

    let first = writer
        .append(message_record("one"), None)
        .expect("appended");
    let second = writer
        .append(message_record("two"), None)
        .expect("appended");

    assert_ne!(first, second);
}

#[test]
fn a_fork_of_a_fork_keeps_every_id_unique() {
    let (_guard, store, _root) = temp_store();
    let (path, head) = {
        let mut writer = store
            .create(new_session(&id(0x0101), Path::new("/tmp")))
            .expect("a created session");
        let head = writer
            .append(message_record("one"), None)
            .expect("appended");
        (writer.path().to_path_buf(), head)
    };

    let first_fork = {
        let mut writer = store.fork(&path, &head, &id(0x0202)).expect("a fork");
        let head = writer
            .append(message_record("two"), None)
            .expect("appended");
        (writer.path().to_path_buf(), head)
    };
    let second_fork = {
        let mut writer = store
            .fork(&first_fork.0, &first_fork.1, &id(0x0303))
            .expect("a fork of a fork");
        writer
            .append(message_record("three"), None)
            .expect("appended");
        writer.path().to_path_buf()
    };

    let ids = ids_in_file(&second_fork);
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "got {ids:?}");
    SessionReader::read(&second_fork).expect("a fork of a fork must read back");
}

#[test]
fn a_resume_of_a_resume_keeps_every_id_unique() {
    let (_guard, store, _root) = temp_store();
    let path = {
        let mut writer = store
            .create(new_session(&id(0x0404), Path::new("/tmp")))
            .expect("a created session");
        writer
            .append(message_record("one"), None)
            .expect("appended");
        writer.path().to_path_buf()
    };
    for round in 0..2 {
        let mut writer = store.append_to(&path).expect("reopened");
        writer
            .append(message_record(&format!("round {round}")), None)
            .expect("appended");
    }

    let ids = ids_in_file(&path);
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "got {ids:?}");
    SessionReader::read(&path).expect("a twice resumed file must read back");
}

#[test]
fn a_reopen_of_a_reopened_file_writes_one_more_reopened_record() {
    let (_guard, store, _root) = temp_store();
    let path = {
        let mut writer = store
            .create(new_session(&id(0x0505), Path::new("/tmp")))
            .expect("a created session");
        writer
            .append(message_record("one"), None)
            .expect("appended");
        writer.close().expect("closed");
        writer.path().to_path_buf()
    };
    {
        let mut writer = store.append_to(&path).expect("reopened");
        writer.close().expect("closed again");
    }
    {
        let _writer = store.append_to(&path).expect("reopened twice");
    }

    let read = SessionReader::read(&path).expect("the file reads back");
    let reopened = read
        .entries
        .iter()
        .filter(|entry| matches!(entry.record, Record::Reopened))
        .count();

    assert_eq!(
        reopened, 2,
        "each reopen of a closed file states itself once"
    );
}

#[test]
fn a_leaf_record_never_becomes_the_chain_head() {
    // `rho sessions name` writes a `Name` leaf. The writer then made it the head, so the next
    // append named a leaf as its parent and the whole file became unreadable:
    //
    //     record r24 names parent r23, which is a leaf record and never a parent
    //
    // A live drive found it, after `sessions name` ran on a session and the next run resumed it.
    // The integrity check of section 6a did its job, and the writer had produced the bad file.
    let (_guard, store, _root) = temp_store();
    let mut writer = store
        .create(new_session(&id(0x0aaa), Path::new("/tmp")))
        .expect("a created session");
    let base = writer.head().expect("the model record");
    let before = writer
        .append(message_record("a prompt"), Some(base))
        .expect("appended");

    let name = writer
        .append(
            Record::Name {
                title: "a title".to_string(),
            },
            Some(before.clone()),
        )
        .expect("a name is written");

    assert_eq!(
        writer.head(),
        Some(before),
        "a leaf must not move the head, or the next record names it as a parent"
    );
    // The next append, exactly as a resume makes.
    let head = writer.head();
    writer
        .append(message_record("after the name"), head)
        .expect("appended");
    let read = SessionReader::read(writer.path()).expect("the file must still read back");
    assert!(
        read.entries.iter().any(|entry| entry.id == name),
        "the name record is still in the file"
    );
}

#[test]
fn a_named_session_reopens_and_stays_readable() {
    // The same defect through the store path, which is how a user meets it: name a session, then
    // resume it.
    let (_guard, store, _root) = temp_store();
    let session = id(0x0bbb);
    let path = {
        let mut writer = store
            .create(new_session(&session, Path::new("/tmp")))
            .expect("a created session");
        let base = writer.head();
        writer
            .append(message_record("a prompt"), base)
            .expect("appended");
        writer.path().to_path_buf()
    };
    {
        let mut writer = store.append_to(&path).expect("reopened to name it");
        let head = writer.head();
        writer
            .append(
                Record::Name {
                    title: "the sign bug".to_string(),
                },
                head,
            )
            .expect("a name is written");
    }
    {
        let mut writer = store.append_to(&path).expect("reopened to continue");
        let head = writer.head();
        writer
            .append(message_record("the next prompt"), head)
            .expect("appended");
    }

    SessionReader::read(&path).expect("a named session must still read back");
}

#[test]
fn a_fork_at_a_leaf_record_leaves_a_usable_head() {
    // A fork copies a branch, and the branch may end at a `Name` leaf. The copy loop made every
    // copied record the head, including the leaf, so the next append through the returned writer
    // named a leaf as its parent and the file was refused.
    //
    // `SessionWriter::append` already skips a leaf. The fork's own copy loop did not, and Codex
    // found the second spelling of the rule.
    let (_guard, store, _root) = temp_store();
    let session = id(0x0ccc);
    let (path, head) = {
        let mut writer = store
            .create(new_session(&session, Path::new("/tmp")))
            .expect("a created session");
        let base = writer.head();
        let message = writer
            .append(message_record("a prompt"), base)
            .expect("appended");
        // The branch ends at a leaf, exactly as `rho sessions name` leaves it.
        let name = writer
            .append(
                Record::Name {
                    title: "a title".to_string(),
                },
                Some(message),
            )
            .expect("a name is written");
        (writer.path().to_path_buf(), name)
    };

    let mut writer = store
        .fork(&path, &head, &id(0x0ddd))
        .expect("a fork at a leaf record");
    let fork_path = writer.path().to_path_buf();
    let next = writer.head();
    writer
        .append(message_record("after the fork"), next)
        .expect("appended");

    SessionReader::read(&fork_path).expect("a fork at a leaf must stay readable");
}

// ---------------------------------------------------------------------------
// Section 7c. An exclusive create, because a collision truncates.
// ---------------------------------------------------------------------------

#[test]
fn create_twice_with_one_id_never_truncates() {
    // `File::create` truncates, and the id is 16 bits of randomness inside one second. At 50
    // concurrent sessions about one run in 53 would erase another session's file in silence.
    let (_guard, store, _root) = temp_store();
    let session = id(0x0606);
    let path = {
        let mut writer = store
            .create(new_session(&session, Path::new("/tmp")))
            .expect("a created session");
        writer
            .append(message_record("the only copy of this message"), None)
            .expect("appended");
        writer.path().to_path_buf()
    };
    let before = std::fs::read_to_string(&path).expect("the file");

    let error = store
        .create(new_session(&session, Path::new("/tmp")))
        .map(|writer| writer.path().to_path_buf())
        .expect_err("a second create with one id must be refused");

    assert!(
        matches!(error, SessionError::Io(_)),
        "expected an io error, got {error:?}"
    );
    let after = std::fs::read_to_string(&path).expect("the file");
    assert_eq!(before, after, "the first file must keep every byte");
}

#[test]
fn create_minted_remints_after_a_collision() {
    // A run calls `create_minted`, which draws its own suffix. Two calls in one millisecond
    // then get two suffixes, so no collision ever happens and a test could never reach the
    // retry. The suffix source is a parameter of `create_minted_from` for exactly that
    // reason, and `create_minted` calls it.
    let (_guard, store, _root) = temp_store();
    let taken = 0x1234u16;
    let (first_id, first) = store
        .create_minted_from(1_756_000_000_000, [taken].into_iter(), without_id())
        .expect("a first session");
    let first_path = first.path().to_path_buf();
    drop(first);
    let before = std::fs::read_to_string(&first_path).expect("the file");

    // The same suffix twice, then a free one. The first two attempts must collide.
    let (second_id, second) = store
        .create_minted_from(
            1_756_000_000_000,
            [taken, taken, 0x5678].into_iter(),
            without_id(),
        )
        .expect("a taken id must cost one more mint, never the session");

    assert_ne!(first_id, second_id, "the retry must mint another id");
    assert_ne!(second.path(), first_path.as_path());
    assert_eq!(
        before,
        std::fs::read_to_string(&first_path).expect("the file"),
        "the first session must keep every byte"
    );
}

#[test]
fn create_minted_gives_up_after_mint_attempts() {
    // A suffix source that repeats one value can never find a free id. The retry is bounded,
    // so a full store cannot spin.
    let (_guard, store, _root) = temp_store();
    let taken = 0x4321u16;
    let (_id, writer) = store
        .create_minted_from(1_756_000_000_000, [taken].into_iter(), without_id())
        .expect("a first session");
    drop(writer);

    let error = store
        .create_minted_from(
            1_756_000_000_000,
            std::iter::repeat_n(taken, rho_core::MINT_ATTEMPTS * 4),
            without_id(),
        )
        .map(|(id, _)| id)
        .expect_err("a store that cannot mint a free id must give up");

    assert!(
        matches!(
            error,
            SessionError::MintExhausted {
                attempts: rho_core::MINT_ATTEMPTS
            }
        ),
        "expected MintExhausted with the bound, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// Section 9. The store is private, by construction.
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_session_file_is_never_briefly_world_readable() {
    // `create_new` then `set_permissions` leaves the file at the umask mode for a moment. Inside a
    // `0o700` store that window is closed by the directory, and the `session-file` config key can
    // name a file in a loose directory where it is not. A security review found it.
    //
    // The mode goes on the `open` call now, so no window exists. The test sets a permissive umask
    // and a world-writable parent, then asserts the mode the file was **created** with.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let loose = dir.path().join("loose");
    std::fs::create_dir_all(&loose).expect("the directory");
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o777)).expect("the mode");
    let path = loose.join("named-session.jsonl");

    let writer = SessionStore::create_file(&path, new_session(&id(0x0e01), Path::new("/tmp")))
        .expect("a created session");

    let mode = std::fs::metadata(writer.path())
        .expect("the file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "a session file must be owner only, got {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn a_session_file_is_0o600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, store, _root) = temp_store();

    let writer = store
        .create(new_session(&id(0x0707), Path::new("/tmp")))
        .expect("a created session");
    let mode = std::fs::metadata(writer.path())
        .expect("the file")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(
        mode, 0o600,
        "a session file must be owner only, got {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn a_store_directory_is_0o700_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Two levels rho must create itself, so the walk is proved and not just the leaf.
    let root = dir.path().join("store").join("project-1234abcd");
    let store = SessionStore::new(&root);

    store
        .create(new_session(&id(0x0808), Path::new("/tmp")))
        .expect("a created session");

    for level in [root.as_path(), root.parent().expect("a parent")] {
        let mode = std::fs::metadata(level)
            .expect("the directory")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode,
            0o700,
            "{} must be owner only, got {mode:o}",
            level.display()
        );
    }
}

#[cfg(unix)]
#[test]
fn a_sidecar_spill_file_is_0o600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, store, _root) = temp_store();
    let mut writer = store
        .create(new_session(&id(0x0909), Path::new("/tmp")))
        .expect("a created session");

    // A record over the cap spills its payload to a sidecar beside the session file.
    writer
        .append(message_record(&"x".repeat(80 * 1024)), None)
        .expect("appended");

    let dir = writer.path().parent().expect("a parent");
    let sidecars: Vec<_> = std::fs::read_dir(dir)
        .expect("the directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("sidecar"))
        .collect();
    assert!(!sidecars.is_empty(), "an oversize record must spill");
    for sidecar in sidecars {
        let mode = std::fs::metadata(&sidecar)
            .expect("the sidecar")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode,
            0o600,
            "{} must be owner only, got {mode:o}",
            sidecar.display()
        );
    }
}
