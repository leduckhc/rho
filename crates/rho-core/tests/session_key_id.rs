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
    // The claim in the name is that the key follows the physical path. A key that is a
    // constant, or that keys on the directory name alone, would still be non-empty. So the
    // old "not empty" assertion proved nothing. This asserts the path is what varies.
    let parent = tempdir().expect("the parent");
    let root = parent.path().join("my-project");
    std::fs::create_dir(&root).expect("the project root");
    let key = ProjectKey::resolve(&root);

    // The name part is the sanitized directory name.
    assert!(
        key.as_str().starts_with("my-project-"),
        "the key {:?} must start with the directory name",
        key.as_str()
    );

    // A directory of the SAME name at a DIFFERENT physical path gets a DIFFERENT key. So
    // the key follows the path, and a fallback to a name-only key would fail here.
    let other_parent = tempdir().expect("another parent");
    let other = other_parent.path().join("my-project");
    std::fs::create_dir(&other).expect("the same-named directory elsewhere");
    let other_key = ProjectKey::resolve(&other);
    assert_ne!(
        key, other_key,
        "two same-named directories on two paths must get two keys"
    );
}

#[test]
fn an_unreadable_git_entry_falls_back_and_never_fails() {
    // The claim in the name is that a junk `.git` file falls back to the physical path. A
    // non-empty key proves nothing about that. So this asserts the key equals the key the
    // same directory gets with no `.git` at all. A fallback to anything else fails.
    let dir = root_with_git_file("\u{0}\u{1}not a git entry at all\u{ff}");
    let with_junk = ProjectKey::resolve(dir.path());

    // Remove the junk `.git`, then resolve the same physical path again.
    std::fs::remove_file(dir.path().join(".git")).expect("remove the junk .git file");
    let plain = ProjectKey::resolve(dir.path());

    assert_eq!(
        with_junk, plain,
        "a junk .git file must key on the physical path, exactly as a directory with no .git"
    );
}

#[test]
fn the_digest_of_a_known_path_never_changes() {
    // Spec section 3b pins the digest: FNV-1a 64-bit over
    // `identity.as_os_str().as_encoded_bytes()`, with the low 32 bits printed as 8 hex
    // digits. The spec states these two values, so a reader can check them by hand.
    //
    // Neither path is read. `resolve` falls back to the physical path when the `.git` entry
    // is absent or is a directory, and the digest is over the path bytes either way. So the
    // real repository at /Users/le/Work/Vibe/rho keys on its own path, and /tmp/example-
    // project needs no directory to exist.
    //
    // This must FAIL today, because `digest_hex` uses `std::hash::DefaultHasher`, whose
    // output Rust does not promise across releases.
    let cases = [
        ("/tmp/example-project", "783befb6"),
        ("/Users/le/Work/Vibe/rho", "2ddec135"),
    ];
    for (path, digest) in cases {
        let key = ProjectKey::resolve(Path::new(path));
        assert!(
            key.as_str().ends_with(digest),
            "the key {:?} for {path:?} must end with the pinned FNV-1a digest {digest:?}; \
             a DefaultHasher digest makes every stored session unreachable after a toolchain \
             upgrade",
            key.as_str()
        );
    }
}

#[test]
fn a_relative_gitdir_shares_the_key_with_the_main_checkout() {
    // Spec section 3c: git writes a relative gitdir when worktree.useRelativePaths is set,
    // and after a worktree moves. A relative gitdir must resolve against the project root
    // before it is hashed, so every worktree of one repository still shares one key.
    //
    // This must FAIL today, because identity_from_gitdir returns the relative parent
    // (`../main-checkout`) unchanged, whose digest differs from the main checkout's path.
    let parent = tempdir().expect("the common parent");
    let main = parent.path().join("main-checkout");
    let worktree = parent.path().join("wt-checkout");
    std::fs::create_dir(&main).expect("the main checkout");
    std::fs::create_dir(&worktree).expect("the worktree");

    // The main checkout holds a real `.git` directory with a worktree entry.
    let git_dir = main.join(".git");
    std::fs::create_dir_all(git_dir.join("worktrees").join("wt")).expect("the worktree entry");

    // The worktree `.git` is a FILE that holds a RELATIVE gitdir line. The `..` climbs out
    // of the worktree to its sibling main checkout.
    std::fs::write(
        worktree.join(".git"),
        "gitdir: ../main-checkout/.git/worktrees/wt\n",
    )
    .expect("the relative worktree .git file");

    let from_main = ProjectKey::resolve(&main);
    let from_worktree = ProjectKey::resolve(&worktree);
    assert_eq!(
        from_main, from_worktree,
        "a relative gitdir must resolve against the worktree root, so both share one key; \
         the walk that returns ../main-checkout unchanged makes this fail"
    );
}

#[test]
fn a_dot_named_project_does_not_hide_the_store() {
    // Spec section 3a and finding m2: a leading dot in the name makes a hidden store
    // directory on unix. A project named `.config` must not vanish from a directory listing.
    //
    // This must FAIL today, because sanitize_name keeps a leading dot.
    let parent = tempdir().expect("the parent");
    let root = parent.path().join(".config");
    std::fs::create_dir(&root).expect("the dot-named project root");
    let key = ProjectKey::resolve(&root);
    assert!(
        !key.as_str().starts_with('.'),
        "a project named .config must not make a hidden store directory; the key {:?} \
         starts with a dot",
        key.as_str()
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

#[test]
fn mint_formats_a_known_instant_in_utc() {
    // A guard for the date arithmetic. Finding M1: no test names a concrete date string, so
    // a break in civil_from_millis shifts both mint calls together and the sort test still
    // passes. Each expected value here was computed by hand from the UTC civil date.
    //
    // This PASSES today. It would fail if civil_from_millis miscomputed a field: a wrong
    // leap-day rule fails the two 2024-02-29 rows, a wrong day carry fails the 23:59:59
    // rows, and a wrong month rollover fails the 2024-03-01 row.
    let cases: [(u64, u16, &str); 5] = [
        (0, 0x0000, "19700101-000000-0000"),
        (1_709_164_800_000, 0x1a2b, "20240229-000000-1a2b"),
        (1_709_251_199_000, 0xffff, "20240229-235959-ffff"),
        (1_735_689_599_000, 0x0abc, "20241231-235959-0abc"),
        (1_709_251_200_000, 0x0001, "20240301-000000-0001"),
    ];
    for (millis, suffix, expected) in cases {
        let id = SessionId::mint(millis, suffix);
        assert_eq!(
            id.as_str(),
            expected,
            "mint({millis}, {suffix:#06x}) must format the exact UTC id"
        );
    }
}

#[test]
fn a_minted_id_parses_back() {
    // A guard, and it also covers SessionId::as_str, which no other test calls, and it
    // round-trips a minted id through parse. Finding M1 and M3.
    //
    // This PASSES today. It would fail if mint produced a shape parse rejects, such as a
    // five-digit year past 9999, or if as_str returned the wrong bytes.
    let id = SessionId::mint(1_709_164_800_000, 0x1a2b);
    let parsed = SessionId::parse(id.as_str()).expect("a freshly minted id must parse");
    assert_eq!(
        parsed.as_str(),
        id.as_str(),
        "parse must round-trip mint, and as_str must return the same bytes"
    );
}
