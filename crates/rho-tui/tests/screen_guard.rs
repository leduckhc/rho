//! Tests for the screen guard. See `SPEC-tui-alternate-screen` section 2 and section 9.
//!
//! The guard must be testable with no real terminal. Each guard here writes to a shared
//! in-memory sink, so a test reads exactly what the guard wrote.

use std::io::{self, Write};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use rho_tui::{ScreenGuard, enter_sequences, restore_sequences};

/// A `Write` sink that shares its buffer, so a test reads it after the guard drops.
#[derive(Clone)]
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl SharedSink {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    /// The bytes written so far, as a string.
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("the sink lock").clone()).expect("utf8 output")
    }
}

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("the sink lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The number of times `needle` appears in `haystack`.
fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn the_guard_enters_the_alternate_screen() {
    assert!(
        enter_sequences(false).contains("\u{1b}[?1049h"),
        "the guard must enter the alternate screen at startup"
    );
}

#[test]
fn the_guard_restores_the_alternate_screen() {
    assert!(
        restore_sequences(false).contains("\u{1b}[?1049l"),
        "the guard must leave the alternate screen on restore"
    );
}

#[test]
fn the_guard_restores_every_mouse_mode() {
    let restore = restore_sequences(true);
    for mode in ["?1000l", "?1002l", "?1003l", "?1006l"] {
        assert!(
            restore.contains(mode),
            "the restore must turn off mouse mode {mode}, got {restore:?}"
        );
    }
}

#[test]
fn a_dropped_guard_restores_the_terminal() {
    let sink = SharedSink::new();
    {
        let _guard =
            ScreenGuard::with_sink(Box::new(sink.clone()), true).expect("the guard enters");
    }
    let output = sink.text();
    assert!(
        output.contains("\u{1b}[?1049l"),
        "a dropped guard must leave the alternate screen, got {output:?}"
    );
    assert!(
        output.contains("\u{1b}[?1000l"),
        "a dropped guard must turn off mouse reporting, got {output:?}"
    );
}

#[test]
fn a_panic_restores_the_terminal() {
    let sink = SharedSink::new();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _guard =
            ScreenGuard::with_sink(Box::new(sink.clone()), true).expect("the guard enters");
        panic!("a run that unwinds through the guard");
    }));
    assert!(result.is_err(), "the panic must propagate");
    let output = sink.text();
    assert!(
        output.contains("\u{1b}[?1049l"),
        "a panic unwinding through the guard must still restore, got {output:?}"
    );
}

#[test]
fn a_restored_guard_does_not_restore_twice() {
    let sink = SharedSink::new();
    {
        let mut guard =
            ScreenGuard::with_sink(Box::new(sink.clone()), true).expect("the guard enters");
        guard.restore().expect("restore succeeds");
        // The guard drops here, and Drop must not write a second restore.
    }
    let output = sink.text();
    assert_eq!(
        count(&output, "\u{1b}[?1049l"),
        1,
        "restore then drop must write the leave sequence once, got {output:?}"
    );
    assert_eq!(
        count(&output, "\u{1b}[?1006l"),
        1,
        "restore then drop must write each mouse reset once, got {output:?}"
    );
}

#[test]
fn the_guard_leaves_and_reenters_for_the_editor() {
    // `app.rs` leaves the terminal for `$EDITOR`, then comes back. `restore` then `reenter`
    // serves that path: the leave sequence, then the enter sequence again.
    let sink = SharedSink::new();
    let mut guard =
        ScreenGuard::with_sink(Box::new(sink.clone()), false).expect("the guard enters");
    guard.restore().expect("leave for the editor");
    guard.reenter().expect("come back from the editor");
    let output = sink.text();

    let leave = output
        .find("\u{1b}[?1049l")
        .expect("the guard left the screen");
    let reenter = output
        .rfind("\u{1b}[?1049h")
        .expect("the guard re-entered the screen");
    assert!(
        leave < reenter,
        "the guard must leave before it re-enters, got {output:?}"
    );

    // A later drop must still restore, because `reenter` armed the guard again.
    drop(guard);
    let output = sink.text();
    assert_eq!(
        count(&output, "\u{1b}[?1049l"),
        2,
        "the editor leave and the final drop each restore once, got {output:?}"
    );
}
