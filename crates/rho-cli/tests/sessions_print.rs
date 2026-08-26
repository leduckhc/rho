//! Tests for the pure `rho sessions` renderers.
//!
//! `rho-cli` is a binary with no library target, so the module is included by path. Every
//! test builds its `ReadResult` and `SessionRow` values by hand, so no test needs a store,
//! a file, a sleep, or the network. See `SPEC-session-store-wiring` section 11.

#[path = "../src/sessions.rs"]
mod sessions;

use rho_core::{
    ContentBlock, Entry, ForkOrigin, Message, ReadResult, Record, RecordId, Role, SessionHeader,
    SessionId, SessionRow, SessionSummary, Usage,
};
use unicode_width::UnicodeWidthStr;

use sessions::{render_list, render_show};

// ---------------------------------------------------------------------------
// Builders. They keep every test short and free of a real store.
// ---------------------------------------------------------------------------

const NOW: u64 = 1_700_000_000_000;

fn id() -> SessionId {
    SessionId::parse("20260825-094512-a3f9").unwrap()
}

fn entry(rid: &str, parent: Option<&str>, millis: u64, record: Record) -> Entry {
    Entry {
        id: RecordId(rid.to_string()),
        parent_id: parent.map(|p| RecordId(p.to_string())),
        timestamp: millis.to_string(),
        record,
    }
}

fn user(text: &str) -> Record {
    Record::Message {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        },
    }
}

fn assistant(text: &str) -> Record {
    Record::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        },
    }
}

fn tool_call(call_id: &str, name: &str, args: serde_json::Value) -> Record {
    Record::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: call_id.to_string(),
                name: name.to_string(),
                arguments: args,
                state: None,
            }],
        },
    }
}

fn tool_result(call_id: &str, body: &str) -> Record {
    Record::Message {
        message: Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: call_id.to_string(),
                content: vec![ContentBlock::Text {
                    text: body.to_string(),
                }],
                is_error: false,
            }],
        },
    }
}

fn read_result(entries: Vec<Entry>) -> ReadResult {
    ReadResult {
        header: SessionHeader {
            version: 1,
            session_id: "20260825-094512-a3f9".to_string(),
            cwd: "/work/rho".into(),
            approval: "ask".to_string(),
            sandbox: "confined".to_string(),
            forked_from: None,
        },
        header_id: RecordId("r0".to_string()),
        entries,
        truncated_tail: false,
        dropped_records: 0,
    }
}

fn summary(id_text: &str, title: &str, model: &str, usage: Option<Usage>) -> SessionRow {
    SessionRow::Session(Box::new(SessionSummary {
        id: SessionId::parse(id_text).unwrap(),
        path: format!("/store/{id_text}.jsonl").into(),
        title: title.to_string(),
        title_is_explicit: false,
        cwd: "/work/rho".into(),
        started_millis: NOW - 120_000,
        last_active_millis: NOW - 120_000,
        size_bytes: 4096,
        model: model.to_string(),
        usage,
        closed: false,
        forked_from: None,
    }))
}

fn some_usage() -> Usage {
    Usage {
        input_tokens: 10_000,
        output_tokens: 4_200,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd: Some(0.08),
    }
}

/// The maximum display width of any line in a rendered block.
fn max_width(text: &str) -> usize {
    text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

#[test]
fn show_prints_one_line_per_record_with_its_id() {
    let entries = vec![
        entry("r1", Some("r0"), NOW, user("fix the parser")),
        entry(
            "r2",
            Some("r1"),
            NOW,
            assistant("I will read the file first."),
        ),
        entry("r3", Some("r2"), NOW, user("yes")),
    ];
    let out = render_show(&read_result(entries), &id(), false);
    let record_lines: Vec<&str> = out.lines().skip(1).collect();
    assert_eq!(record_lines.len(), 3, "one line per record");
    for (rid, line) in ["r1", "r2", "r3"].iter().zip(&record_lines) {
        let first = line.split_whitespace().next().unwrap();
        assert_eq!(&first, rid, "the record id is the first column");
    }
}

#[test]
fn show_fits_eighty_columns() {
    // The invariant over long text and over wide unicode, not one example.
    let long_ascii = "a".repeat(5000);
    let wide_unicode = "字".repeat(300);
    let entries = vec![
        entry("r1", Some("r0"), NOW, user(&long_ascii)),
        entry("r2", Some("r1"), NOW, assistant(&wide_unicode)),
        entry(
            "r3",
            Some("r2"),
            NOW,
            user(&format!("{long_ascii}{wide_unicode}")),
        ),
    ];
    let out = render_show(&read_result(entries), &id(), false);
    assert!(
        max_width(&out) <= 80,
        "no line exceeds 80 columns, got {}",
        max_width(&out)
    );
}

#[test]
fn show_cuts_a_long_text_and_never_wraps() {
    let long = "a".repeat(5000);
    let entries = vec![entry("r1", Some("r0"), NOW, user(&long))];
    let out = render_show(&read_result(entries), &id(), false);
    let record_line = out.lines().nth(1).unwrap();
    assert!(!record_line.contains('\n'), "the record prints as one line");
    assert!(
        UnicodeWidthStr::width(record_line) <= 80,
        "the one line fits 80 columns"
    );
    assert!(record_line.contains('…'), "the cut is marked");
}

#[test]
fn show_names_a_tool_and_never_prints_a_result_body() {
    let secret = "SECRET_TOKEN_ax7Qz";
    let entries = vec![
        entry(
            "r1",
            Some("r0"),
            NOW,
            tool_call("c1", "read", serde_json::json!({"path": "src/parse.rs"})),
        ),
        entry(
            "r2",
            Some("r1"),
            NOW,
            tool_result("c1", &format!("api_key={secret}")),
        ),
    ];
    let out = render_show(&read_result(entries), &id(), false);
    assert!(out.contains("read"), "the tool is named");
    assert!(out.contains("tool_result"), "the result kind is shown");
    assert!(out.contains(" B"), "a byte count is shown");
    assert!(
        !out.contains(secret),
        "the result body never reaches the terminal"
    );
}

#[test]
fn show_marks_a_sibling_branch() {
    // r2 and r3 both answer r1, so they are two branches of one question.
    let entries = vec![
        entry("r1", Some("r0"), NOW, user("which fix?")),
        entry("r2", Some("r1"), NOW, assistant("fix A")),
        entry("r3", Some("r1"), NOW, assistant("fix B")),
    ];
    let out = render_show(&read_result(entries), &id(), false);
    let r3_line = out.lines().find(|l| l.contains("fix B")).unwrap();
    assert!(
        r3_line.contains('+'),
        "the sibling branch is marked, got {r3_line:?}"
    );
    let r2_line = out.lines().find(|l| l.contains("fix A")).unwrap();
    assert!(!r2_line.contains('+'), "the first branch is not marked");
}

#[test]
fn show_full_prints_the_whole_text() {
    let long = "b".repeat(5000);
    let entries = vec![entry("r1", Some("r0"), NOW, user(&long))];
    let out = render_show(&read_result(entries), &id(), true);
    assert!(out.contains(&long), "full prints the whole text");
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn list_prints_six_columns_inside_eighty() {
    let rows = vec![
        summary(
            "20260825-094512-a3f9",
            "fix the parser",
            "claude-sonnet-4",
            Some(some_usage()),
        ),
        summary(
            "20260824-171003-77b2",
            "add the retry test",
            "claude-haiku-4",
            Some(some_usage()),
        ),
    ];
    let out = render_list(&rows, NOW, false);
    let header = out.lines().next().unwrap();
    for name in ["ID", "LAST ACTIVE", "TITLE", "MODEL", "TOKENS", "COST"] {
        assert!(header.contains(name), "the header names {name}");
    }
    assert!(
        max_width(&out) <= 80,
        "every line fits 80 columns, got {}",
        max_width(&out)
    );
}

#[test]
fn list_shows_an_unreadable_row_with_its_reason() {
    let rows = vec![SessionRow::Unreadable {
        path: "/store/20260823-092211-0c41.jsonl".into(),
        reason: "bad header".to_string(),
    }];
    let out = render_list(&rows, NOW, false);
    let row = out.lines().nth(1).unwrap();
    assert!(
        row.contains("20260823-092211-0c41"),
        "the id from the stem survives"
    );
    assert!(
        row.contains("* unreadable: bad header"),
        "the reason survives"
    );
    assert!(row.contains('-'), "an unknown field shows a dash");
    assert!(UnicodeWidthStr::width(row) <= 80, "the row fits 80 columns");
}

#[test]
fn list_shows_no_turn_count() {
    let rows = vec![summary(
        "20260825-094512-a3f9",
        "fix the parser",
        "claude-sonnet-4",
        Some(some_usage()),
    )];
    let out = render_list(&rows, NOW, false);
    assert!(
        !out.to_lowercase().contains("turn"),
        "no column reports a turn count"
    );
}

#[test]
fn list_cuts_a_long_title_with_an_ellipsis() {
    let long_title = "fix the parser and also the lexer and the whole pipeline end to end";
    let rows = vec![summary(
        "20260825-094512-a3f9",
        long_title,
        "claude-sonnet-4",
        Some(some_usage()),
    )];
    let out = render_list(&rows, NOW, false);
    let row = out.lines().nth(1).unwrap();
    assert!(row.contains('…'), "a long title is cut with an ellipsis");
    assert!(
        !row.contains("end to end"),
        "the tail of the title is not printed"
    );
    assert!(
        UnicodeWidthStr::width(row) <= 80,
        "the row still fits 80 columns"
    );
}

#[test]
fn list_shows_a_dash_for_a_session_with_no_usage() {
    let rows = vec![summary(
        "20260825-094512-a3f9",
        "no turn yet",
        "claude-sonnet-4",
        None,
    )];
    let out = render_list(&rows, NOW, false);
    let row = out.lines().nth(1).unwrap();
    // The tokens and cost columns are the last two, so both trailing fields are a dash.
    let tail: Vec<&str> = row.split_whitespace().collect();
    let last_two = &tail[tail.len() - 2..];
    assert_eq!(
        last_two,
        &["-", "-"],
        "no usage shows a dash for tokens and cost"
    );
}

#[test]
fn list_long_adds_the_directory_and_the_fork_origin() {
    let mut row = summary(
        "20260825-094512-a3f9",
        "fix the parser",
        "claude-sonnet-4",
        Some(some_usage()),
    );
    if let SessionRow::Session(ref mut s) = row {
        s.cwd = "/work/rho/worktree".into();
        s.forked_from = Some(ForkOrigin {
            session_id: "20260101-000000-1111".to_string(),
            record_id: RecordId("r7".to_string()),
        });
    }
    let rows = vec![row];
    let long = render_list(&rows, NOW, true);
    assert!(
        long.contains("/work/rho/worktree"),
        "long adds the working directory"
    );
    assert!(
        long.contains("20260101-000000-1111"),
        "long adds the fork origin session"
    );
    assert!(long.contains("r7"), "long adds the fork origin record");

    let short = render_list(&rows, NOW, false);
    assert!(
        !short.contains("/work/rho/worktree"),
        "the short list adds no directory"
    );
    assert!(
        !short.contains("20260101-000000-1111"),
        "the short list adds no fork origin"
    );
}
