//! The `RHO_PROGRESS` line protocol and progress inference.
//!
//! A child reports progress by writing a line to stdout or stderr in this form:
//!
//! ```text
//! RHO_PROGRESS {"percent": 42, "message": "compiling", "done": 6, "total": 10}
//! ```
//!
//! A line that starts with the prefix and parses as JSON becomes a progress
//! event and leaves the captured output. A malformed progress line stays in the
//! output and fails nothing. When a child writes no explicit progress, rho infers
//! progress from common shapes such as `6/10`, `42%`, and `[3 of 7]`.
//!
//! A progress `message` is untrusted input, because a file the child prints can
//! hold anything. So the message is sanitised here, before it becomes an event.

use std::sync::LazyLock;

use regex::Regex;
use rho_core::TaskProgress;

/// The sentinel prefix. A line that starts with it opts in to structured
/// progress. A shell script writes one with a single `echo`.
pub const PROGRESS_PREFIX: &str = "RHO_PROGRESS ";

/// The result of scanning one output line for progress.
#[derive(Debug, PartialEq)]
pub enum ProgressScan {
    /// The line is an explicit progress line. Remove it from the output and
    /// report the progress.
    Explicit(TaskProgress),
    /// The line is ordinary output. Keep it. It may carry inferred progress.
    Output { inferred: Option<TaskProgress> },
}

/// Scan one output line. An explicit `RHO_PROGRESS` line that parses as JSON is
/// consumed. Every other line stays in the output.
pub fn scan_line(line: &str) -> ProgressScan {
    if let Some(rest) = line.strip_prefix(PROGRESS_PREFIX) {
        match parse_progress_json(rest) {
            // A valid progress line is consumed.
            Some(progress) => return ProgressScan::Explicit(progress),
            // A malformed progress line stays in the output and raises no error.
            None => return ProgressScan::Output { inferred: None },
        }
    }
    ProgressScan::Output {
        inferred: infer_progress(line),
    }
}

/// Parse the JSON body of an explicit progress line. Return `None` when the body
/// is not a JSON object, so the caller keeps the line in the output.
fn parse_progress_json(body: &str) -> Option<TaskProgress> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let object = value.as_object()?;
    let percent = object
        .get("percent")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value.min(100) as u8);
    let message = object
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(sanitize_message);
    let done = object.get("done").and_then(serde_json::Value::as_u64);
    let total = object.get("total").and_then(serde_json::Value::as_u64);
    Some(TaskProgress {
        percent,
        message,
        done,
        total,
    })
}

/// Infer progress from a common count or percent shape. Return `None` when the
/// line matches no shape.
fn infer_progress(line: &str) -> Option<TaskProgress> {
    // A `[3 of 7]` shape. Check it before the bare count, because it is stricter.
    if let Some(captures) = OF_SHAPE.captures(line) {
        let done = captures[1].parse().ok();
        let total = captures[2].parse().ok();
        if let (Some(done), Some(total)) = (done, total) {
            return Some(TaskProgress {
                done: Some(done),
                total: Some(total),
                ..Default::default()
            });
        }
    }
    // A `6/10` count shape.
    if let Some(captures) = COUNT_SHAPE.captures(line) {
        let done = captures[1].parse().ok();
        let total = captures[2].parse().ok();
        if let (Some(done), Some(total)) = (done, total) {
            return Some(TaskProgress {
                done: Some(done),
                total: Some(total),
                ..Default::default()
            });
        }
    }
    // A `42%` percent shape.
    if let Some(captures) = PERCENT_SHAPE.captures(line)
        && let Ok(percent) = captures[1].parse::<u64>()
    {
        return Some(TaskProgress {
            percent: Some(percent.min(100) as u8),
            ..Default::default()
        });
    }
    None
}

/// Remove control characters and terminal escape sequences from a progress
/// message. A progress message is untrusted, so a control sequence must not reach
/// the display. This mirrors the tool-output rule in `SPEC-07` section 9.
pub fn sanitize_message(input: &str) -> String {
    let without_escapes = ANSI_ESCAPE.replace_all(input, "");
    without_escapes
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

/// A terminal escape sequence, including a bare `ESC` followed by one byte.
static ANSI_ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b.").expect("the regex is valid"));
/// A `[3 of 7]` shape.
static OF_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\s*(\d+)\s+of\s+(\d+)\s*\]").expect("the regex is valid"));
/// A `6/10` count shape.
static COUNT_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d+)\s*/\s*(\d+)\b").expect("the regex is valid"));
/// A `42%` percent shape.
static PERCENT_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,3})\s*%").expect("the regex is valid"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_progress_line_parses_and_is_consumed() {
        let scan = scan_line(r#"RHO_PROGRESS {"percent": 42, "message": "compiling"}"#);
        assert_eq!(
            scan,
            ProgressScan::Explicit(TaskProgress {
                percent: Some(42),
                message: Some("compiling".to_string()),
                done: None,
                total: None,
            })
        );
    }

    #[test]
    fn malformed_progress_line_is_kept_as_output() {
        let scan = scan_line("RHO_PROGRESS not json at all");
        assert_eq!(scan, ProgressScan::Output { inferred: None });
    }

    #[test]
    fn percent_is_clamped_to_one_hundred() {
        let scan = scan_line(r#"RHO_PROGRESS {"percent": 250}"#);
        match scan {
            ProgressScan::Explicit(progress) => assert_eq!(progress.percent, Some(100)),
            other => panic!("expected explicit progress, got {other:?}"),
        }
    }

    #[test]
    fn count_shape_is_inferred() {
        let scan = scan_line("compiled 6/10 crates");
        assert_eq!(
            scan,
            ProgressScan::Output {
                inferred: Some(TaskProgress {
                    done: Some(6),
                    total: Some(10),
                    ..Default::default()
                })
            }
        );
    }

    #[test]
    fn of_shape_is_inferred() {
        let scan = scan_line("step [3 of 7] running");
        assert_eq!(
            scan,
            ProgressScan::Output {
                inferred: Some(TaskProgress {
                    done: Some(3),
                    total: Some(7),
                    ..Default::default()
                })
            }
        );
    }

    #[test]
    fn percent_shape_is_inferred() {
        let scan = scan_line("progress: 42%");
        assert_eq!(
            scan,
            ProgressScan::Output {
                inferred: Some(TaskProgress {
                    percent: Some(42),
                    ..Default::default()
                })
            }
        );
    }

    #[test]
    fn progress_message_with_an_escape_sequence_is_sanitised() {
        // A real child JSON-escapes the escape byte as `\u001b`. The sanitiser
        // then removes the decoded escape sequence.
        let scan = scan_line(r#"RHO_PROGRESS {"message": "a\u001b[31mred\u001b[0m b"}"#);
        match scan {
            ProgressScan::Explicit(progress) => {
                let message = progress.message.unwrap();
                assert!(!message.contains('\u{1b}'), "no escape byte: {message:?}");
                assert_eq!(message, "ared b");
            }
            other => panic!("expected explicit progress, got {other:?}"),
        }
    }
}
