//! Render the `rho sessions` text surfaces.
//!
//! This module is pure. Each function takes data and returns a `String`. A test reads the
//! text and never a terminal. See `SPEC-session-store-wiring` sections 8a and 8b.
//!
//! Two rules matter for safety. A tool result never prints its body, so a secret in a
//! result never reaches the terminal. A long text is cut at the width and never wrapped, so
//! one record is always one line.

use std::collections::{HashMap, HashSet};

use rho_core::{ContentBlock, Entry, ReadResult, Record, Role, SessionId, SessionRow, Usage};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The width every printed line must fit inside.
const LINE_WIDTH: usize = 80;

// The `sessions list` columns. The six widths plus five single-space gaps sum to 80.
const LIST_ID_W: usize = 20;
const LIST_ACTIVE_W: usize = 11;
const LIST_TITLE_W: usize = 18;
const LIST_MODEL_W: usize = 15;
const LIST_TOKENS_W: usize = 6;
const LIST_COST_W: usize = 5;

/// The mark that names a field rho did not read.
const DASH: &str = "-";

/// The prefix that names a row rho could not read.
const UNREADABLE_PREFIX: &str = "* unreadable: ";

/// Render the `rho sessions list` table. `now_millis` makes the relative time deterministic.
pub fn render_list(rows: &[SessionRow], now_millis: u64, long: bool) -> String {
    let mut out = String::new();
    out.push_str(&list_header(long));
    for row in rows {
        out.push('\n');
        out.push_str(&list_row(row, now_millis, long));
    }
    // A trailing newline, so a shell prompt does not run into the last row.
    out.push('\n');
    out
}

/// Build the header line of the list.
fn list_header(long: bool) -> String {
    let mut line = join_columns(&[
        col("ID", LIST_ID_W, false),
        col("LAST ACTIVE", LIST_ACTIVE_W, false),
        col("TITLE", LIST_TITLE_W, false),
        col("MODEL", LIST_MODEL_W, false),
        col("TOKENS", LIST_TOKENS_W, true),
        col("COST", LIST_COST_W, true),
    ]);
    if long {
        line.push_str("  CWD  FORK");
    }
    line
}

/// Build one row of the list.
fn list_row(row: &SessionRow, now_millis: u64, long: bool) -> String {
    match row {
        SessionRow::Session(summary) => {
            let (tokens, cost) = match &summary.usage {
                Some(usage) => (fmt_tokens(usage), fmt_cost(usage)),
                None => (DASH.to_string(), DASH.to_string()),
            };
            let mut line = join_columns(&[
                col(summary.id.as_str(), LIST_ID_W, false),
                col(
                    &relative_time(now_millis, summary.last_active_millis),
                    LIST_ACTIVE_W,
                    false,
                ),
                col(&summary.title, LIST_TITLE_W, false),
                col(&summary.model, LIST_MODEL_W, false),
                col(&tokens, LIST_TOKENS_W, true),
                col(&cost, LIST_COST_W, true),
            ]);
            if long {
                line.push_str("  ");
                line.push_str(&summary.cwd.display().to_string());
                line.push_str("  ");
                match &summary.forked_from {
                    Some(origin) => {
                        line.push_str(&format!("fork:{}@{}", origin.session_id, origin.record_id));
                    }
                    None => line.push_str(DASH),
                }
            }
            line
        }
        SessionRow::Unreadable { path, reason } => {
            // An unreadable row has only a path, so the id comes from the file stem.
            let id = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let reason_text = format!("{UNREADABLE_PREFIX}{reason}");
            // The reason spans the title and model columns, so a full reason survives.
            let reason_w = LIST_TITLE_W + 1 + LIST_MODEL_W;
            join_columns(&[
                col(&id, LIST_ID_W, false),
                col(DASH, LIST_ACTIVE_W, false),
                col(&reason_text, reason_w, false),
                col(DASH, LIST_TOKENS_W, true),
                col(DASH, LIST_COST_W, true),
            ])
        }
    }
}

/// Render the `rho sessions show` listing for one session.
pub fn render_show(read: &ReadResult, id: &SessionId, full: bool) -> String {
    let mut out = show_header(read, id);
    let names = tool_names(&read.entries);
    let siblings = sibling_ids(&read.entries);
    let depths = depths(&read.entries);
    for entry in &read.entries {
        out.push('\n');
        let depth = depths.get(&entry.id.0).copied().unwrap_or(0);
        let is_sibling = siblings.contains(&entry.id.0);
        out.push_str(&show_row(entry, depth, is_sibling, &names, full));
    }
    // A trailing newline, so a shell prompt does not run into the last row. A live drive found
    // the missing one.
    out.push('\n');
    out
}

/// Build the one-line header of a show listing.
fn show_header(read: &ReadResult, id: &SessionId) -> String {
    let title = show_title(&read.entries);
    let model = show_model(&read.entries);
    let closed = if last_is_closed(&read.entries) {
        "closed"
    } else {
        "open"
    };
    // **The state never gives way.** It is one word, and it is the only field a user cannot read
    // again from a record line, so it is the last thing cut.
    //
    // The title gives way first, because the first prompt line repeats it. Then the model gives
    // way, because a real Bedrock id is 41 characters and it pushed the state off the end. A live
    // drive found the first case, and a test reviewer found the second.
    let frame = format!("session  {}  \"\"    {closed}", id.as_str());
    let room = LINE_WIDTH.saturating_sub(display_width(&frame));
    // The model keeps at most half of what is left, so a long model can never starve the title and
    // a long title can never starve the model.
    let model = fit(&model, room / 2);
    let title = fit(&title, room.saturating_sub(display_width(&model)));
    let line = format!("session  {}  \"{title}\"  {model}  {closed}", id.as_str());
    fit(&line, LINE_WIDTH)
}

/// Build one record line of a show listing.
fn show_row(
    entry: &Entry,
    depth: usize,
    is_sibling: bool,
    names: &HashMap<String, String>,
    full: bool,
) -> String {
    let indent = if is_sibling {
        // A sibling replaces the last indent level with a mark, so its width is unchanged.
        let mut p = "  ".repeat(depth);
        p.push_str("+ ");
        p
    } else {
        "  ".repeat(depth + 1)
    };
    let (kind, detail) = describe_record(&entry.record, names);
    let (h, m, s) = time_of_day(parse_millis(&entry.timestamp));
    let time = format!("{h:02}:{m:02}:{s:02}");
    let left = format!(
        "{indent}{}  {time}  {}  ",
        pad_right(&entry.id.0, 4),
        pad_right(&kind, 11)
    );
    if full {
        return format!("{left}{detail}");
    }
    let used = display_width(&left);
    let remaining = LINE_WIDTH.saturating_sub(used);
    format!("{left}{}", fit(&detail, remaining))
}

/// Name the kind and the short detail of one record.
///
/// A tool result names the tool and a byte count, never the content.
fn describe_record(record: &Record, names: &HashMap<String, String>) -> (String, String) {
    match record {
        Record::Session { .. } => ("session".to_string(), String::new()),
        Record::ModelChange { provider, model } => {
            ("model".to_string(), format!("{provider} {model}"))
        }
        Record::Message { message } => describe_message(&message.role, &message.content, names),
        Record::Usage { usage } => ("usage".to_string(), fmt_tokens(usage)),
        Record::Stop { stop_reason } => ("stop".to_string(), format!("{stop_reason:?}")),
        Record::Closed => ("closed".to_string(), String::new()),
        Record::Reopened => ("reopened".to_string(), String::new()),
        Record::Name { title } => ("name".to_string(), title.clone()),
    }
}

/// Name the kind and the detail of one message record.
fn describe_message(
    role: &Role,
    content: &[ContentBlock],
    names: &HashMap<String, String>,
) -> (String, String) {
    // **A tool result is the safety case, and the rule is the role.** Name the tool and the byte
    // count, never the body. A secret inside a result must never reach the terminal by accident,
    // and `docs/guide/sessions.md` promises that.
    //
    // A first version keyed on the block shape, so a tool message holding a bare `Text` block fell
    // through to the text arm and printed the body. A crafted or imported file holds exactly that,
    // and a security review found it.
    if matches!(role, Role::Tool) {
        let name = content
            .iter()
            .find_map(|block| match block {
                ContentBlock::ToolResult { tool_call_id, .. } => names
                    .get(tool_call_id)
                    .cloned()
                    .or_else(|| Some(tool_call_id.clone())),
                _ => None,
            })
            .unwrap_or_else(|| "a tool".to_string());
        let bytes = result_bytes(content);
        return (
            "tool_result".to_string(),
            format!("{name}  {}", fmt_bytes(bytes)),
        );
    }
    for block in content {
        if let ContentBlock::ToolResult {
            tool_call_id,
            content,
            ..
        } = block
        {
            let name = names
                .get(tool_call_id)
                .cloned()
                .unwrap_or_else(|| tool_call_id.clone());
            let bytes = result_bytes(content);
            return (
                "tool_result".to_string(),
                format!("{name}  {}", fmt_bytes(bytes)),
            );
        }
    }
    for block in content {
        if let ContentBlock::ToolCall {
            name, arguments, ..
        } = block
        {
            return (
                "tool_call".to_string(),
                format!("{name}  {}", short_args(arguments)),
            );
        }
    }
    let text = joined_text(content);
    match role {
        Role::User => ("user".to_string(), text),
        Role::Assistant => ("assistant".to_string(), text),
        Role::Tool => ("tool".to_string(), text),
    }
}

/// The bytes of a tool result, so the row states a size and never the content.
fn result_bytes(content: &[ContentBlock]) -> usize {
    let mut total = 0;
    for block in content {
        match block {
            ContentBlock::Text { text }
            | ContentBlock::ReasoningTrace { text }
            | ContentBlock::ReasoningReplay { text, .. } => total += text.len(),
            ContentBlock::ToolResult { content, .. } => total += result_bytes(content),
            _ => {}
        }
    }
    total
}

/// The first line of the joined text of a message.
fn joined_text(content: &[ContentBlock]) -> String {
    let mut text = String::new();
    for block in content {
        if let ContentBlock::Text { text: t } = block {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t);
        }
    }
    text
}

/// The short arguments of a tool call, as `key=value` pairs.
fn short_args(arguments: &serde_json::Value) -> String {
    match arguments.as_object() {
        Some(map) => map
            .iter()
            .map(|(k, v)| format!("{k}={}", scalar(v)))
            .collect::<Vec<_>>()
            .join(" "),
        None => scalar(arguments),
    }
}

/// One JSON value as a short scalar, with no surrounding quotes.
fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Map every tool call id to its tool name, so a result can name its tool.
fn tool_names(entries: &[Entry]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in entries {
        if let Record::Message { message } = &entry.record {
            for block in &message.content {
                if let ContentBlock::ToolCall { id, name, .. } = block {
                    map.insert(id.clone(), name.clone());
                }
            }
        }
    }
    map
}

/// The ids of records that are a later child of a parent with more than one child.
fn sibling_ids(entries: &[Entry]) -> std::collections::HashSet<String> {
    let mut seen_first: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut siblings = std::collections::HashSet::new();
    // Count children per parent first.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        if let Some(pid) = &entry.parent_id {
            *counts.entry(pid.to_string()).or_insert(0) += 1;
        }
    }
    for entry in entries {
        if let Some(pid) = &entry.parent_id {
            let key = pid.to_string();
            if counts.get(&key).copied().unwrap_or(0) > 1 && !seen_first.insert(key) {
                // The parent was seen before, so this record is a later child.
                siblings.insert(entry.id.to_string());
            }
        }
    }
    siblings
}

/// The indent depth of every record, counted from ancestors inside the entry set.
fn depths(entries: &[Entry]) -> HashMap<String, usize> {
    // **The depth counts branch points, not chain length.**
    //
    // A first version counted the whole chain to the root. A live drive then showed a linear
    // conversation as a staircase: every record sat one level deeper than the one before it, and
    // the text ran off the 80 column line after six records. See
    // `docs/verification/session-store-wiring.md` section 6.
    //
    // Indentation exists to show a **branch**. So a record goes one level deeper only when it
    // descends from a record that has more than one child. A conversation with no fork then
    // prints flat, and a fork prints one level in.
    let mut children: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        if let Some(parent) = &entry.parent_id {
            *children.entry(parent.to_string()).or_insert(0) += 1;
        }
    }
    let mut by_id: HashMap<String, &Entry> = HashMap::new();
    for entry in entries {
        by_id.insert(entry.id.to_string(), entry);
    }
    // **Memoised, so the walk is linear.** A first version walked to the root for every record, so a
    // long session cost O(N squared), and a security review named that a denial of service. A
    // record's depth is its parent's depth, plus one when the parent has more than one child.
    //
    // The chain cannot be cyclic here, because `SessionReader::read` refuses a cyclic file. The
    // `seen` set stays anyway, so a caller with hand-built entries cannot spin.
    let mut depths: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        // Walk up to the first record whose depth is known, then fill the path back down.
        let mut path: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut cursor = entry.id.to_string();
        let mut base = 0usize;
        loop {
            if let Some(known) = depths.get(&cursor) {
                base = *known;
                break;
            }
            if !seen.insert(cursor.clone()) {
                break;
            }
            path.push(cursor.clone());
            match by_id.get(&cursor).and_then(|e| e.parent_id.as_ref()) {
                Some(parent) => cursor = parent.to_string(),
                // A root, or a parent this listing does not hold.
                None => break,
            }
        }
        // `path` runs child first, so fill it in reverse.
        for id in path.iter().rev() {
            let parent_branches = by_id
                .get(id)
                .and_then(|e| e.parent_id.as_ref())
                .map(|parent| children.get(&parent.to_string()).copied().unwrap_or(0) > 1)
                .unwrap_or(false);
            base += usize::from(parent_branches);
            depths.insert(id.clone(), base);
        }
    }
    depths
}

/// The title for the show header, from the newest name or the first prompt.
fn show_title(entries: &[Entry]) -> String {
    for entry in entries.iter().rev() {
        if let Record::Name { title } = &entry.record {
            return title.clone();
        }
    }
    for entry in entries {
        if let Record::Message { message } = &entry.record
            && message.role == Role::User
        {
            let text = joined_text(&message.content);
            return text.lines().next().unwrap_or("").to_string();
        }
    }
    String::new()
}

/// The model for the show header, from the newest model change.
fn show_model(entries: &[Entry]) -> String {
    for entry in entries.iter().rev() {
        if let Record::ModelChange { model, .. } = &entry.record {
            return model.clone();
        }
    }
    DASH.to_string()
}

/// True when the last record closed the session.
fn last_is_closed(entries: &[Entry]) -> bool {
    matches!(entries.last().map(|e| &e.record), Some(Record::Closed))
}

// ---------------------------------------------------------------------------
// Small formatting helpers.
// ---------------------------------------------------------------------------

/// Parse a decimal epoch-milliseconds string. A bad value reads as zero.
fn parse_millis(text: &str) -> u64 {
    text.parse().unwrap_or(0)
}

/// The UTC time of day for an epoch-milliseconds value, by integer arithmetic.
///
/// rho-core has no date dependency, so this adds none. It returns the hour, the minute, and
/// the second only.
fn time_of_day(epoch_millis: u64) -> (u32, u32, u32) {
    const SECONDS_PER_DAY: i64 = 86_400;
    let total_seconds = (epoch_millis / 1000) as i64;
    let seconds_of_day = total_seconds.rem_euclid(SECONDS_PER_DAY);
    let hour = (seconds_of_day / 3600) as u32;
    let minute = ((seconds_of_day % 3600) / 60) as u32;
    let second = (seconds_of_day % 60) as u32;
    (hour, minute, second)
}

/// A short relative time, such as `2 min ago` or `yesterday`.
fn relative_time(now_millis: u64, then_millis: u64) -> String {
    let diff = now_millis.saturating_sub(then_millis);
    let seconds = diff / 1000;
    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 3600 {
        format!("{} min ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} hr ago", seconds / 3600)
    } else if seconds < 172_800 {
        "yesterday".to_string()
    } else {
        format!("{} days ago", seconds / 86_400)
    }
}

/// Short token total, such as `14.2k`.
fn fmt_tokens(usage: &Usage) -> String {
    let total = usage.input_tokens + usage.output_tokens;
    if total < 1000 {
        total.to_string()
    } else if total < 1_000_000 {
        format!("{:.1}k", total as f64 / 1000.0)
    } else {
        format!("{:.1}M", total as f64 / 1_000_000.0)
    }
}

/// Short cost, such as `$0.08`. A missing cost is a dash.
fn fmt_cost(usage: &Usage) -> String {
    match usage.cost_usd {
        Some(cost) => format!("${cost:.2}"),
        None => DASH.to_string(),
    }
}

/// A human byte count, such as `1.2 KiB`.
fn fmt_bytes(n: usize) -> String {
    const KIB: f64 = 1024.0;
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if f < KIB * KIB {
        format!("{:.1} KiB", f / KIB)
    } else {
        format!("{:.1} MiB", f / (KIB * KIB))
    }
}

/// The display width of a string, which counts a wide character as two columns.
fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Cut a string to a display width, and mark a cut with a single ellipsis.
fn fit(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // The ellipsis is one column, so the kept text fits in `max - 1`.
    let budget = max - 1;
    let mut out = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > budget {
            break;
        }
        out.push(ch);
        width += cw;
    }
    out.push('…');
    out
}

/// Fit a string to a column, then pad it to that column width.
fn col(s: &str, width: usize, right: bool) -> String {
    let cut = fit(s, width);
    if right {
        pad_left(&cut, width)
    } else {
        pad_right(&cut, width)
    }
}

/// Pad a string on the right to a display width.
fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

/// Pad a string on the left to a display width.
fn pad_left(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - w))
    }
}

/// Join columns with a single space.
fn join_columns(columns: &[String]) -> String {
    columns.join(" ")
}
