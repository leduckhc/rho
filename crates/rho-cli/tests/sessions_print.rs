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

fn model_change(model: &str) -> Record {
    Record::ModelChange {
        provider: "bedrock".to_string(),
        model: model.to_string(),
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

// ---------------------------------------------------------------------------
// What a live drive of the real binary found. See
// `docs/verification/session-store-wiring.md` section 6.
// ---------------------------------------------------------------------------

#[test]
fn show_does_not_indent_a_linear_conversation() {
    // A first version counted the whole chain to the root, so a linear conversation printed as a
    // staircase and the text ran off the line after six records. Indentation exists to show a
    // branch, so a conversation with no fork prints flat.
    let read = read_result(vec![
        entry("r1", Some("r0"), NOW, user("the first prompt")),
        entry("r2", Some("r1"), NOW, assistant("the first answer")),
        entry("r3", Some("r2"), NOW, user("the second prompt")),
        entry("r4", Some("r3"), NOW, assistant("the second answer")),
        entry("r5", Some("r4"), NOW, user("the third prompt")),
        entry("r6", Some("r5"), NOW, assistant("the third answer")),
    ]);

    let text = render_show(&read, &id(), false);

    let columns: Vec<usize> = text
        .lines()
        .skip(1)
        .map(|line| line.find('r').expect("a record id"))
        .collect();
    assert_eq!(
        columns,
        vec![2, 2, 2, 2, 2, 2],
        "a chain with no fork must print flat, got\n{text}"
    );
}

#[test]
fn show_indents_only_below_a_branch_point() {
    // Two answers to one question. Both descend from a record with two children, so both go one
    // level in, and the later one is marked.
    let read = read_result(vec![
        entry("r1", Some("r0"), NOW, user("one question")),
        entry("r2", Some("r1"), NOW, assistant("the first answer")),
        entry("r3", Some("r1"), NOW, assistant("the second answer")),
    ]);

    let text = render_show(&read, &id(), false);

    // The column the record id starts in. A sibling replaces the last indent level with a mark,
    // so its leading spaces are fewer while its id sits in the same column.
    let columns: Vec<usize> = text
        .lines()
        .skip(1)
        .map(|line| line.find('r').expect("a record id"))
        .collect();
    assert_eq!(
        columns,
        vec![2, 4, 4],
        "only a record below a branch point is indented, got\n{text}"
    );
    assert!(
        text.lines().any(|line| line.trim_start().starts_with("+ ")),
        "the later of two answers to one question is marked, got\n{text}"
    );
}

#[test]
fn both_renderers_end_with_a_newline() {
    // A live drive showed the shell prompt running into the last row.
    let read = read_result(vec![entry("r1", Some("r0"), NOW, user("a prompt"))]);
    assert!(render_show(&read, &id(), false).ends_with('\n'));

    let rows = vec![summary("20260825-094512-a3f9", "a title", "m", None)];
    assert!(render_list(&rows, 1_756_000_000_000, false).ends_with('\n'));
}

#[test]
fn show_keeps_the_model_and_the_state_when_the_title_is_long() {
    // A live drive showed a long title pushing the model and the close state off the header. The
    // title gives way, because a user can read it again on the first prompt line.
    let read = read_result(vec![
        entry(
            "r1",
            Some("r0"),
            NOW,
            model_change("a-very-long-model-identifier"),
        ),
        entry("r2", Some("r1"), NOW, user(&"t".repeat(300))),
        entry("r3", Some("r2"), NOW, Record::Closed),
    ]);

    let text = render_show(&read, &id(), false);
    let header = text.lines().next().expect("a header");

    // The model is recognisable, and not necessarily whole. Eighty columns cannot hold a 300
    // character title, a 28 character model, and the state, so both fields give way and the state
    // never does. `show_keeps_the_state_when_the_model_is_long` pins the other half.
    assert!(
        header.contains("a-very-long-model"),
        "the model must stay recognisable under a long title, got {header}"
    );
    assert!(
        header.ends_with("closed"),
        "the close state must survive a long title, got {header}"
    );
    assert!(
        header.chars().count() <= 80,
        "got {} columns",
        header.chars().count()
    );
}

#[test]
fn show_keeps_the_state_when_the_model_is_long() {
    // A real Bedrock or Claude model id is long. The header cut from the end, so the close state
    // became `c…` and the header then lied about whether the session was open.
    //
    // A test reviewer reproduced it with `anthropic.claude-3-5-sonnet-20241022-v2:0`. The earlier
    // test only stressed a long **title**, which gives way by design, so nothing saw this.
    let read = read_result(vec![
        entry(
            "r1",
            Some("r0"),
            NOW,
            model_change("anthropic.claude-3-5-sonnet-20241022-v2:0"),
        ),
        entry("r2", Some("r1"), NOW, user("fix the parser")),
        entry("r3", Some("r2"), NOW, Record::Closed),
    ]);

    let text = render_show(&read, &id(), false);
    let header = text.lines().next().expect("a header");

    assert!(
        header.ends_with("closed"),
        "the close state must survive a long model, got {header}"
    );
    assert!(
        header.contains("anthropic.claude"),
        "the model must still be recognisable, got {header}"
    );
    assert!(
        header.chars().count() <= 80,
        "got {} columns: {header}",
        header.chars().count()
    );
}

#[test]
fn show_of_a_long_session_is_not_quadratic() {
    // `depths` walked to the root for every record, so a long session cost O(N squared). A security
    // review named it. The walk memoises now, so the cost is linear.
    //
    // The assertion is a wall clock with a generous bound, because the defect is a growth rate and
    // a tight number would be flaky. Four thousand records at O(N squared) is eight million steps
    // of map lookups and clones, which takes seconds; linear takes milliseconds.
    let mut entries = Vec::new();
    for n in 0..4000u32 {
        entries.push(entry(
            &format!("r{}", n + 1),
            Some(&if n == 0 {
                "r0".to_string()
            } else {
                format!("r{n}")
            }),
            NOW,
            user("a prompt"),
        ));
    }
    let read = read_result(entries);

    let started = std::time::Instant::now();
    let text = render_show(&read, &id(), false);
    let elapsed = started.elapsed();

    assert_eq!(text.lines().count(), 4001, "every record is one line");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "a linear walk must render 4000 records well inside two seconds, it took {elapsed:?}"
    );
    println!("4000 records in {elapsed:?}");
}

#[test]
fn show_never_prints_a_tool_message_body() {
    // A tool message whose content is a bare `Text` block, with no `ToolResult` wrapper, fell to
    // the text arm and **printed the body**. A crafted or imported file can hold exactly that, and
    // `docs/guide/sessions.md` promises that a secret inside a result does not reach the terminal.
    //
    // A security review found it. The rule is the role, and not the block shape: a message from a
    // tool is evidence, and `show` reports its size.
    let read = read_result(vec![entry(
        "r1",
        Some("r0"),
        NOW,
        Record::Message {
            message: Message {
                role: Role::Tool,
                content: vec![ContentBlock::Text {
                    text: "AWS_SECRET_ACCESS_KEY=must-not-be-printed".to_string(),
                }],
            },
        },
    )]);

    let text = render_show(&read, &id(), false);

    assert!(
        !text.contains("must-not-be-printed"),
        "a tool message body must never reach the terminal, got {text}"
    );
    assert!(
        text.contains("tool_result"),
        "the row still says what kind of record it is, got {text}"
    );
}

#[test]
fn show_full_never_prints_a_tool_message_body_either() {
    // `--full` prints the whole text of each record. That must not turn the safety case off, or the
    // flag becomes a way to read every secret a session recorded.
    let read = read_result(vec![entry(
        "r1",
        Some("r0"),
        NOW,
        Record::Message {
            message: Message {
                role: Role::Tool,
                content: vec![ContentBlock::Text {
                    text: "AWS_SECRET_ACCESS_KEY=must-not-be-printed".to_string(),
                }],
            },
        },
    )]);

    let text = render_show(&read, &id(), true);

    assert!(
        !text.contains("must-not-be-printed"),
        "--full must not print a tool message body, got {text}"
    );
}
