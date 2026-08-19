//! The terminal screen guard. Owned by `rho-tui`. See `SPEC-tui-alternate-screen`.
//!
//! Restoration is structural. A statement at the end of `run` does not survive a panic or
//! a signal. A measured `SIGTERM` left mouse reporting on, so the shell printed
//! `35;111;18M` on every mouse move. So the guard restores the terminal three ways: `Drop`
//! on any unwind, a panic hook before the panic message prints, and a signal handler.
//! See `D-alternate-screen-after-all`.

use std::io::{self, Write};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use crate::app::TuiError;

/// Enter the alternate screen. `?1049h` gives rho its own buffer.
const ENTER_ALT_SCREEN: &str = "\u{1b}[?1049h";
/// Leave the alternate screen. `?1049l` gives the shell its buffer back.
const LEAVE_ALT_SCREEN: &str = "\u{1b}[?1049l";
/// Turn on button, drag, motion, and SGR mouse reporting.
const ENABLE_MOUSE: &str = "\u{1b}[?1000h\u{1b}[?1002h\u{1b}[?1003h\u{1b}[?1006h";
/// Turn off SGR, motion, drag, and button mouse reporting, in reverse order.
const DISABLE_MOUSE: &str = "\u{1b}[?1006l\u{1b}[?1003l\u{1b}[?1002l\u{1b}[?1000l";

/// The exact sequences the guard writes at entry.
///
/// It enters the alternate screen, and it adds mouse reporting when `mouse` is true.
/// `bench/check-sequences.py` greps for each, so a future edit cannot drop one silently.
pub fn enter_sequences(mouse: bool) -> String {
    let mut out = String::from(ENTER_ALT_SCREEN);
    if mouse {
        out.push_str(ENABLE_MOUSE);
    }
    out
}

/// The exact sequences the guard writes to restore the terminal.
///
/// It turns off mouse reporting first, then leaves the alternate screen. It must contain
/// `?1049l`, and every mouse reset when `mouse` is true, so a killed rho leaves no mode on.
pub fn restore_sequences(mouse: bool) -> String {
    let mut out = String::new();
    if mouse {
        out.push_str(DISABLE_MOUSE);
    }
    out.push_str(LEAVE_ALT_SCREEN);
    out
}

/// Owns the terminal modes for the lifetime of the UI.
///
/// `Drop` restores every mode, so a panic that unwinds through `run` still leaves the
/// terminal usable. Construction is the only way to enter the alternate screen.
pub struct ScreenGuard {
    /// Where the guard writes its control sequences. Real runs write standard output. A
    /// test injects a sink, so it reads exactly what the guard wrote with no terminal.
    out: Box<dyn Write + Send>,
    /// Whether the guard also toggles raw mode. A test sink toggles none, so it needs no
    /// terminal.
    manage_raw: bool,
    mouse: bool,
    restored: bool,
}

impl ScreenGuard {
    /// Enter raw mode, the alternate screen, and mouse reporting when `mouse` is true.
    ///
    /// It also installs a panic hook that restores the terminal before the panic message
    /// prints. A panic message printed into raw mode is unreadable.
    pub fn enter(mouse: bool) -> Result<Self, TuiError> {
        enable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
        install_panic_hook(mouse);
        let mut guard = Self {
            out: Box::new(io::stdout()),
            manage_raw: true,
            mouse,
            restored: false,
        };
        guard.write_enter()?;
        Ok(guard)
    }

    /// Build a guard that writes to a caller sink, for a test with no terminal.
    ///
    /// It manages no raw mode and installs no panic hook, so a test reads exactly the
    /// sequences the guard wrote.
    pub fn with_sink(out: Box<dyn Write + Send>, mouse: bool) -> Result<Self, TuiError> {
        let mut guard = Self {
            out,
            manage_raw: false,
            mouse,
            restored: false,
        };
        guard.write_enter()?;
        Ok(guard)
    }

    /// Restore every mode now, and make `Drop` a no-op.
    ///
    /// It writes the sequences once. A second call writes nothing, because a double reset
    /// clears a line the shell has already drawn. The editor path calls it to leave rho.
    pub fn restore(&mut self) -> Result<(), TuiError> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        self.write(&restore_sequences(self.mouse))?;
        if self.manage_raw {
            disable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
        }
        Ok(())
    }

    /// Enter the alternate screen again after `restore`.
    ///
    /// The editor path calls it to come back to rho. It arms `Drop` again, so a later exit
    /// still restores.
    pub fn reenter(&mut self) -> Result<(), TuiError> {
        if self.manage_raw {
            enable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
        }
        self.restored = false;
        self.write_enter()
    }

    /// Write the entry sequences to the sink.
    fn write_enter(&mut self) -> Result<(), TuiError> {
        self.write(&enter_sequences(self.mouse))
    }

    /// Write one sequence to the sink, then flush it.
    fn write(&mut self, sequence: &str) -> Result<(), TuiError> {
        self.out
            .write_all(sequence.as_bytes())
            .and_then(|()| self.out.flush())
            .map_err(|error| TuiError::Io(error.to_string()))
    }
}

impl Drop for ScreenGuard {
    /// Restore on every exit path, including a panic that unwinds through `run`.
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Install a panic hook that restores the terminal before the panic message prints.
///
/// A panic message printed into raw mode is unreadable. The hook restores standard output,
/// then calls the previous hook, so the message still reaches the user. See the ratatui
/// recipe: <https://ratatui.rs/recipes/apps/panic-hooks/>.
fn install_panic_hook(mouse: bool) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut stdout = io::stdout();
        let _ = stdout.write_all(restore_sequences(mouse).as_bytes());
        let _ = stdout.flush();
        let _ = disable_raw_mode();
        previous(info);
    }));
}

/// Restore the terminal on a fatal signal, then exit.
///
/// It waits for `SIGTERM`, `SIGHUP`, or `SIGINT`. `SIGHUP` is what closing a window sends.
/// A killed rho must not leave mouse reporting on. `process::exit` runs no destructor, so
/// the guard's `Drop` does not also write the sequences, and the shell reads no double
/// reset. The caller aborts the returned task when `run` ends cleanly.
pub fn spawn_signal_restore(mouse: bool) -> tokio::task::JoinHandle<()> {
    use tokio::signal::unix::{SignalKind, signal};
    tokio::spawn(async move {
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(_) => return,
        };
        let mut hangup = match signal(SignalKind::hangup()) {
            Ok(stream) => stream,
            Err(_) => return,
        };
        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(stream) => stream,
            Err(_) => return,
        };
        tokio::select! {
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
            _ = interrupt.recv() => {}
        }
        let mut stdout = io::stdout();
        let _ = stdout.write_all(restore_sequences(mouse).as_bytes());
        let _ = stdout.flush();
        let _ = disable_raw_mode();
        std::process::exit(130);
    })
}
