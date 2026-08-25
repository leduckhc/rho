//! `SPEC-session-store-wiring` stage `core-red`, slice 1: the project key and the session id.
//!
//! These tests drive the contract in section 3, 3a, and 4. Every test must fail on an
//! unimplemented body, never on a type error. It isolates the filesystem with `tempfile`.
//! It never pauses, never touches the network, and never reads the real home directory.
//!
//! Note on the prefix tests. Section 11 lists `a_unique_prefix_resolves_to_one_session`,
//! `an_ambiguous_prefix_lists_every_match`, `an_unknown_prefix_resolves_to_none`, and
//! `an_unknown_prefix_on_the_command_line_names_the_project` under "The id". Each one needs
//! `SessionStore::resolve_prefix` (spec section 7) or the command line (crate `rho-cli`).
//! Both are outside sections 3, 3a, and 4, so they belong to a later slice. See the
//! handover findings.

use std::io::{self, BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rho_core::{GIT_ENTRY_MAX_BYTES, ProjectKey, SessionError, SessionId, default_store_root};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// A byte source that counts the bytes it hands out.
//
// It sits under the reader, so a full forward scan is counted, not hidden by a buffer.
// A correct `resolve_from` reads at most `GIT_ENTRY_MAX_BYTES` and stops. A wrong one that
// reads the whole file and then truncates the string makes the count exceed the cap.
// ---------------------------------------------------------------------------
struct CountingReader {
    data: Vec<u8>,
    pos: usize,
    handed_out: Arc<AtomicUsize>,
}

impl CountingReader {
    fn new(data: Vec<u8>) -> (Self, Arc<AtomicUsize>) {
        let handed_out = Arc::new(AtomicUsize::new(0));
        let reader = CountingReader {
            data,
            pos: 0,
            handed_out: Arc::clone(&handed_out),
        };
        (reader, handed_out)
    }
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.data.len() - self.pos;
        let n = remaining.min(buf.len());
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        self.handed_out.fetch_add(n, Ordering::SeqCst);
        Ok(n)
    }
}

impl BufRead for CountingReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        // Hand back at most a small chunk, as a real `BufReader` does. A borrow is not
        // "handed out"; `consume` counts what the reader takes. Bounding the slice forces
        // any reader that wants more than 512 bytes to call `consume` and be counted, so a
        // reader cannot borrow the whole file in one `fill_buf` and bypass the counter.
        let end = (self.pos + 512).min(self.data.len());
        Ok(&self.data[self.pos..end])
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt;
        self.handed_out.fetch_add(amt, Ordering::SeqCst);
    }
}

/// Make a project root directory that holds a `.git` file with the given content.
fn root_with_git_file(content: &str) -> tempfile::TempDir {
    let dir = tempdir().expect("a temp project root");
    std::fs::write(dir.path().join(".git"), content).expect("write the .git file");
    dir
}

// ---------------------------------------------------------------------------
// The project key.
// ---------------------------------------------------------------------------

#[test]
fn every_worktree_of_one_repository_shares_a_key() {
    // The main checkout holds a real `.git` directory.
    let main = tempdir().expect("the main checkout");
    let git_dir = main.path().join(".git");
    std::fs::create_dir(&git_dir).expect("the .git directory");
    std::fs::create_dir_all(git_dir.join("worktrees").join("wt")).expect("the worktree entry");

    // The worktree holds a `.git` file that names the main gitdir.
    let worktree = tempdir().expect("the worktree");
    let gitdir = git_dir.join("worktrees").join("wt");
    std::fs::write(
        worktree.path().join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )
    .expect("write the worktree .git file");

    let from_main = ProjectKey::resolve(main.path());
    let from_worktree = ProjectKey::resolve(worktree.path());
    assert_eq!(
        from_main, from_worktree,
        "every worktree of one repository shares one key"
    );
}

#[test]
fn a_plain_directory_keys_on_its_own_path() {
    let dir = tempdir().expect("a plain project root");
    let key = ProjectKey::resolve(dir.path());
    assert!(
        !key.as_str().is_empty(),
        "a directory with no .git still resolves to a key"
    );
}

#[test]
fn an_unreadable_git_entry_falls_back_and_never_fails() {
    let dir = root_with_git_file("\u{0}\u{1}not a git entry at all\u{ff}");
    let key = ProjectKey::resolve(dir.path());
    assert!(
        !key.as_str().is_empty(),
        "a junk .git file falls back to the physical path and never fails"
    );
}

#[test]
fn two_directories_of_one_name_get_two_keys() {
    // Two roots share a directory name, under two different parents.
    let parent_a = tempdir().expect("parent a");
    let parent_b = tempdir().expect("parent b");
    let a = parent_a.path().join("proj");
    let b = parent_b.path().join("proj");
    std::fs::create_dir(&a).expect("root a");
    std::fs::create_dir(&b).expect("root b");

    let key_a = ProjectKey::resolve(&a);
    let key_b = ProjectKey::resolve(&b);
    assert_ne!(
        key_a, key_b,
        "the digest of the identity path keeps two same-named directories apart"
    );
}

#[test]
fn a_hostile_gitdir_cannot_escape_the_store() {
    let store = tempdir().expect("the store root");
    let store_root: PathBuf = store.path().join("sessions");

    // A table of hostile `gitdir:` lines. The invariant, not one example.
    let hostile = [
        "gitdir: ../../../../etc\n",
        "gitdir: ../../../../../../tmp/evil\n",
        "gitdir: /etc/passwd\n",
        "gitdir: ..\\..\\..\\windows\n",
        "gitdir: subdir/../../../escape\n",
        "gitdir: ....//....//etc/shadow\n",
        "gitdir: /\n",
        "gitdir: ../\n",
    ];

    for line in hostile {
        let dir = root_with_git_file(line);
        let key = ProjectKey::resolve(dir.path());
        let name = key.as_str();

        // The key is one path segment, so the join can never leave the store root.
        assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "a hostile gitdir line {line:?} yielded an unsafe key segment {name:?}"
        );
        let joined = store_root.join(name);
        assert!(
            joined.starts_with(&store_root),
            "a hostile gitdir line {line:?} escaped the store: {joined:?}"
        );
    }
}

#[test]
fn a_key_is_always_one_path_segment() {
    let contents = [
        "gitdir: /path/to/main/.git/worktrees/name\n",
        "gitdir: ../../../../etc\n",
        "not a gitdir line at all",
        "",
        "gitdir: a/b/c/d\n",
    ];
    for content in contents {
        let dir = root_with_git_file(content);
        let key = ProjectKey::resolve(dir.path());
        let name = key.as_str();
        assert_eq!(
            Path::new(name).components().count(),
            1,
            "for .git content {content:?} the key {name:?} is not one segment"
        );
        assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "the key {name:?} holds a separator or a parent reference"
        );
    }
}

#[test]
fn a_giant_git_entry_stops_at_the_cap() {
    // A one-line `.git` file of ten megabytes with no newline.
    let giant = vec![b'a'; 10 * 1024 * 1024];
    let (reader, handed_out) = CountingReader::new(giant);
    let root = tempdir().expect("a project root");

    // The seam. A correct implementation reads at most `GIT_ENTRY_MAX_BYTES`.
    let _key = ProjectKey::resolve_from(reader, root.path());

    let bytes = handed_out.load(Ordering::SeqCst);
    assert!(
        bytes <= GIT_ENTRY_MAX_BYTES,
        "the seam handed out {bytes} bytes, past the cap of {GIT_ENTRY_MAX_BYTES}; \
         an implementation that reads the whole file and then truncates fails here"
    );
}

#[test]
fn a_key_that_sanitizes_to_nothing_gets_a_placeholder() {
    // A directory whose name holds only bytes outside `[A-Za-z0-9._-]`.
    let parent = tempdir().expect("the parent");
    let root = parent.path().join("@@@@");
    std::fs::create_dir(&root).expect("the illegal-named root");

    let key = ProjectKey::resolve(&root);
    let name = key.as_str();
    assert_eq!(
        Path::new(name).components().count(),
        1,
        "a name of only illegal bytes still yields one segment, not an empty one"
    );
    assert!(
        !name.starts_with('-'),
        "the sanitized name became empty, so the key {name:?} starts with the hex separator"
    );
}

#[test]
fn the_default_store_root_sits_under_the_passed_home() {
    let home = tempdir().expect("a fake home");
    let root = default_store_root(home.path());
    assert_eq!(
        root,
        home.path().join(".rho").join("sessions"),
        "the store root sits under the passed home, never the real one"
    );
}

// ---------------------------------------------------------------------------
// The id.
// ---------------------------------------------------------------------------

#[test]
fn an_id_sorts_by_time() {
    let older = SessionId::mint(1_000, 0xabcd);
    let newer = SessionId::mint(2_000, 0x0001);
    assert!(
        newer > older,
        "the newer mint time must sort after the older, so a name sort reads newest first"
    );
}

#[test]
fn two_sessions_in_one_second_get_two_ids() {
    let one = SessionId::mint(1_000, 0x0001);
    let two = SessionId::mint(1_000, 0x0002);
    assert_ne!(
        one, two,
        "the same stamp with two suffixes must produce two ids"
    );
}

#[test]
fn a_malformed_id_is_refused() {
    let result = SessionId::parse("this-is-not-a-session-id");
    assert!(
        matches!(result, Err(SessionError::Decode(_))),
        "a name that is not the id shape must be a Decode error, not an accepted id"
    );
}
