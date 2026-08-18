//! The pure renderer.
//!
//! The renderer draws the state into a `ratatui` frame. It is a pure function of
//! the state and the frame area. It does no IO, and it reads no clock: every
//! duration and the motion sweep come from the tick and the spans the state
//! carries. A test renders into a `TestBackend` and asserts on the buffer. See
//! `SPEC-tui-experience` and `docs/tui-design.md`.
//!
//! The draw path writes strings straight into the frame buffer, one full-width
//! row at a time, rather than building a `Vec` of styled lines per frame. That is
//! the low-allocation path the cost budget marks as this stage's target: a cell
//! that already exists is overwritten, so no per-cell or per-span value is
//! allocated. See `docs/benchmarks.md`.

use ratatui::Frame;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::bindings::{bindings, filter_slash_commands};
use crate::concise::{RowFold, fold_caret};
use crate::duration::{duration_slot, format_duration};
use crate::motion::{MotionCell, motion_cell, sweep_weight};
use crate::sanitize::sanitize_line;
use crate::state::{ActivityState, Approval, Panel, Row, SlashList, ToolRowStatus, TuiState};
use crate::theme::{Role, role_256};

/// The brand mark. The `ρ` renders in the accent role, so it is the first accent
/// on screen.
const BRAND: &str = "ρ rho";
/// The composer placeholder, shown in `muted` when the draft is empty.
const PLACEHOLDER: &str = "Type a prompt. / for commands. ? for help.";
/// The verbatim approval choices row. There is no allow-always choice, because
/// `D-no-remembered-execute-allow` forbids a remembered execute approval.
const APPROVAL_CHOICES: &str = "[y] allow once    [n] deny    [esc] deny and cancel the turn";
/// The commands line of the help panel.
const HELP_COMMANDS: &str = "type / to list them · /guide is the two minute tour";

/// The widest transcript measure. Text wraps to `min(TEXT_MEASURE_CAP, width - MARGIN)`,
/// so a wide terminal keeps a readable line length.
const TEXT_MEASURE_CAP: usize = 80;
/// The columns the transcript measure leaves off the frame width.
const TEXT_MEASURE_MARGIN: usize = 10;
/// The columns a footer or panel keeps clear at each edge.
const EDGE_MARGIN: usize = 2;

/// One glyph and its meaning. The glyph tier here is the UTF-8 tier from the design.
const GLYPH_USER: &str = "❯";
const GLYPH_THINKING: &str = "∴";
const GLYPH_ERROR: &str = "✗";
const GLYPH_ACTIVITY: &str = "◈";
const GLYPH_APPROVAL: &str = "!";
const GLYPH_SEPARATOR: &str = "·";
const GLYPH_CURSOR: &str = "█";

/// Draw the whole UI. Pure. No IO. Safe to call every frame.
pub fn render(state: &TuiState, frame: &mut Frame<'_>) {
    let area = frame.area();
    let width = area.width as usize;
    let height = area.height as usize;
    if width == 0 || height == 0 {
        return;
    }

    // Build the movable regions, then decide which survive at this height. The
    // composer is a border pair around one or more input rows.
    let composer = composer_lines(state, width);
    let input_rows = composer.len().saturating_sub(2);
    let panel = panel_lines(state, width);
    let panel_rows = panel.len();

    // Decide the layout for this height. Chrome drops in a stated order as the
    // frame shrinks: the rule first, then the header, then the footer, then the
    // composer border. The composer input line and the transcript are the last two
    // survivors, and at a height of one only the transcript renders. See the
    // degradation order in `SPEC-tui-experience` (added by the controller).
    let layout = plan_layout(height, input_rows, panel_rows);

    // Write the surviving regions top to bottom, straight into the buffer. No
    // intermediate line is cloned, so the frame allocates only the strings it
    // draws once, keeping the per-frame cost under the measured baseline.
    let mut y = 0usize;
    if layout.header {
        put(frame, y, width, &header_line(state, width), text_style());
        y += 1;
    }
    if layout.rule {
        put(frame, y, width, &rule_line(width), style_for(Role::Muted));
        y += 1;
    }
    let transcript = if state.rows.is_empty() && state.panel == Panel::None {
        empty_state(state, width, layout.transcript_rows)
    } else {
        transcript_window(state, width, layout.transcript_rows)
    };
    for offset in 0..layout.transcript_rows {
        match transcript.get(offset) {
            Some((text, style)) => put(frame, y, width, text, *style),
            None => put(frame, y, width, &blank(width), text_style()),
        }
        y += 1;
    }
    if layout.panel {
        for (text, style) in &panel {
            put(frame, y, width, text, *style);
            y += 1;
        }
    }
    if layout.composer_border {
        let (text, style) = &composer[0];
        put(frame, y, width, text, *style);
        y += 1;
    }
    if layout.composer_input {
        for (text, style) in &composer[1..composer.len() - 1] {
            put(frame, y, width, text, *style);
            y += 1;
        }
    }
    if layout.composer_border {
        let (text, style) = &composer[composer.len() - 1];
        put(frame, y, width, text, *style);
        y += 1;
    }
    if layout.footer {
        let (footer, word_at, hint_at) = footer_line(state, width);
        // The hints are muted, and the activity word is text. One muted row for both made
        // `done · end turn` as faint as the hints, and a user reported it as invisible.
        put(frame, y, width, &footer, text_style());
        restyle(frame, y, hint_at, width, style_for(Role::Muted));
        apply_sweep(state, frame, y, word_at);
    }
}

/// The slash-command index drawn at screen row `row`, if any.
///
/// A mouse click carries a row, and the row means nothing without the layout. So this
/// asks the same `plan_layout` the renderer uses, which keeps one source of truth for
/// the geometry. It returns `None` when the click misses the list, when no list is
/// open, or when the frame is too short to draw the panel.
pub fn slash_row_index(state: &TuiState, width: u16, height: u16, row: u16) -> Option<usize> {
    let Panel::SlashList(list) = &state.panel else {
        return None;
    };
    let count = filter_slash_commands(&list.query).len();
    if count == 0 {
        return None;
    }
    let composer = composer_lines(state, width as usize);
    let input_rows = composer.len().saturating_sub(2);
    // The panel is its two rules plus one row for each command.
    let layout = plan_layout(height as usize, input_rows, count + 2);
    if !layout.panel {
        return None;
    }
    let mut first = 0usize;
    if layout.header {
        first += 1;
    }
    if layout.rule {
        first += 1;
    }
    first += layout.transcript_rows;
    // The panel opens with a rule, so the first command sits one row lower.
    let first_command = first + 1;
    let clicked = row as usize;
    if clicked >= first_command && clicked < first_command + count {
        Some(clicked - first_command)
    } else {
        None
    }
}

/// Which regions survive at a given height, and how many rows the transcript gets.
struct Layout {
    header: bool,
    rule: bool,
    panel: bool,
    composer_border: bool,
    composer_input: bool,
    footer: bool,
    transcript_rows: usize,
}

/// Plan the layout for one height. The transcript always keeps at least one row
/// while the frame has a row. Chrome is added back in keep-priority order, the
/// reverse of the drop order, so a taller frame simply keeps more of it. A region
/// that cannot fit is skipped, and its rows fall through to the transcript.
fn plan_layout(height: usize, input_rows: usize, panel_rows: usize) -> Layout {
    // One row is reserved for the transcript; the rest is handed out by priority.
    let mut remaining = height.saturating_sub(1);
    let mut take = |cost: usize| -> bool {
        if cost > 0 && remaining >= cost {
            remaining -= cost;
            true
        } else {
            false
        }
    };
    // Keep-priority, highest first: input, border, footer, header, rule, panel.
    let composer_input = take(input_rows);
    let composer_border = take(2);
    let footer = take(1);
    let header = take(1);
    let rule = take(1);
    let panel = take(panel_rows);
    Layout {
        header,
        rule,
        panel,
        composer_border,
        composer_input,
        footer,
        transcript_rows: 1 + remaining,
    }
}

// ---- The header. ----------------------------------------------------------

/// The one header content row. The brand and the session clock survive at every
/// width. The metadata drops right to left as the terminal narrows.
fn header_line(state: &TuiState, width: usize) -> String {
    let (left, meta) = if width >= 100 {
        (
            format!("{BRAND}   {} {GLYPH_SEPARATOR} {}", state.cwd, state.branch),
            format!(
                "{} {GLYPH_SEPARATOR} {} {GLYPH_SEPARATOR} {}",
                state.model, state.provider, state.tokens
            ),
        )
    } else if width >= 80 {
        (
            format!("{BRAND}   {} {GLYPH_SEPARATOR} {}", state.cwd, state.branch),
            format!("{} {GLYPH_SEPARATOR} {}", state.model, state.tokens),
        )
    } else {
        (BRAND.to_string(), state.model.clone())
    };
    let right = format!("{meta}   {}", duration_slot(state.session_millis));
    justify(&left, &right, width)
}

// ---- The transcript. ------------------------------------------------------

/// Build the visible transcript window: the last `rows` lines, top-aligned. A blank
/// row separates turns, except that a run of tool rows stays together. The walk
/// starts at the newest row and stops once the window is full, so an unbounded
/// transcript costs no more per frame than a screenful.
fn transcript_window(state: &TuiState, width: usize, rows: usize) -> Vec<(String, Style)> {
    let measure = TEXT_MEASURE_CAP.min(width.saturating_sub(TEXT_MEASURE_MARGIN));
    let mut blocks: Vec<Vec<(String, Style)>> = Vec::new();
    let mut total = 0usize;
    for index in (0..state.rows.len()).rev() {
        let row = &state.rows[index];
        let is_tool = matches!(row, Row::Tool { .. });
        let prev_tool = index > 0 && matches!(state.rows[index - 1], Row::Tool { .. });
        let mut block: Vec<(String, Style)> = Vec::new();
        if !(is_tool && prev_tool) {
            block.push((blank(width), text_style()));
        }
        push_row(&mut block, state, index, row, width, measure);
        total += block.len();
        blocks.push(block);
        if total >= rows {
            break;
        }
    }
    blocks.reverse();
    let mut lines: Vec<(String, Style)> = blocks.into_iter().flatten().collect();
    // Drop any overflow from the top, so the newest row stays visible.
    if lines.len() > rows {
        lines.drain(0..lines.len() - rows);
    }
    lines
}

/// Render one transcript row into its lines.
fn push_row(
    out: &mut Vec<(String, Style)>,
    state: &TuiState,
    index: usize,
    row: &Row,
    width: usize,
    measure: usize,
) {
    match row {
        Row::User { text } => {
            let wrapped = wrap(&sanitize_line(text), measure.saturating_sub(2));
            for (line_index, line) in wrapped.iter().enumerate() {
                let body = if line_index == 0 {
                    format!("{GLYPH_USER} {line}")
                } else {
                    format!("  {line}")
                };
                out.push((pad(&body, width), text_style()));
            }
        }
        Row::Assistant { text } => {
            let wrapped = wrap(&sanitize_line(text), measure);
            for line in wrapped {
                out.push((pad(&line, width), text_style()));
            }
        }
        Row::Thinking { text: _ } => {
            let body = match format_duration(row_duration(state, index)) {
                Some(span) => format!("{GLYPH_THINKING} thought for {span}"),
                None => format!("{GLYPH_THINKING} thinking"),
            };
            out.push((pad(&body, width), style_for(Role::Muted)));
        }
        Row::Tool {
            name,
            status,
            preview,
            ..
        } => {
            out.push((
                tool_header(state, index, name, preview, *status, width),
                text_style(),
            ));
            if row_fold(state, index) == RowFold::Expanded {
                for line in tool_body(state, index, preview) {
                    out.push((
                        pad(&format!("    {}", sanitize_line(&line)), width),
                        style_for(Role::Muted),
                    ));
                }
            }
        }
        Row::Error { message, detail } => {
            out.push((
                pad(
                    &format!(
                        "{GLYPH_ERROR} error {GLYPH_SEPARATOR} {}",
                        sanitize_line(message)
                    ),
                    width,
                ),
                style_for(Role::Error),
            ));
            for line in detail {
                out.push((
                    pad(&format!("     {}", sanitize_line(line)), width),
                    style_for(Role::Muted),
                ));
            }
        }
        Row::Agent { name, outcome, .. } => {
            out.push((
                pad(
                    &format!("agent {} {}", sanitize_line(name), sanitize_line(outcome)),
                    width,
                ),
                text_style(),
            ));
        }
        Row::Task {
            command,
            state: task,
            ..
        } => {
            out.push((
                pad(
                    &format!("task {} {}", sanitize_line(command), sanitize_line(task)),
                    width,
                ),
                text_style(),
            ));
        }
    }
}

/// The concise tool header: verb, payload, duration slot, status glyph, and caret.
fn tool_header(
    state: &TuiState,
    index: usize,
    name: &str,
    payload: &str,
    status: ToolRowStatus,
    width: usize,
) -> String {
    let left = format!("{}  {}", name, sanitize_line(payload));
    let right = format!(
        "{} {} {}",
        duration_slot(row_duration(state, index)),
        status_glyph(status),
        fold_caret(row_fold(state, index)),
    );
    justify(&left, &right, width)
}

/// The expanded body lines of a tool row.
fn tool_body(state: &TuiState, index: usize, preview: &str) -> Vec<String> {
    match state.row_bodies.get(index) {
        Some(body) if !body.is_empty() => body.clone(),
        _ => preview.lines().map(str::to_string).collect(),
    }
}

// ---- The empty state. -----------------------------------------------------

/// The four rows of half-block brand art, in accent.
const BRAND_ART: [&str; 4] = ["▄▀▀▄", "█  █", "█▄▄▀", "█"];
/// The starter lines: each names one key and one outcome.
const STARTERS: [(&str, &str); 4] = [
    ("❯", "type a prompt to begin"),
    ("/", "list the commands"),
    ("?", "show the keys"),
    ("/guide", "take the two minute tour"),
];
/// The wordmark row.
const WORDMARK: &str = "rho · the harness, unbundled";
/// The display width of the starter block, the widest starter line.
const STARTER_BLOCK_WIDTH: usize = 32;

/// The first-launch frame: a centred block over the transcript area. It returns
/// exactly `rows` lines, so the caller places it with no further arithmetic.
fn empty_state(state: &TuiState, width: usize, rows: usize) -> Vec<(String, Style)> {
    let accent = style_for(Role::Accent);
    let muted = style_for(Role::Muted);
    let art_indent = width.saturating_sub(4) / 2;
    let starter_indent = width.saturating_sub(STARTER_BLOCK_WIDTH) / 2;

    let mut block: Vec<(String, Style)> = Vec::new();
    for art in BRAND_ART {
        block.push((
            pad(&format!("{}{art}", " ".repeat(art_indent)), width),
            accent,
        ));
    }
    block.push((blank(width), text_style()));
    block.push((centre(WORDMARK, width), text_style()));
    block.push((blank(width), text_style()));
    let session = format!(
        "{} {GLYPH_SEPARATOR} {} {GLYPH_SEPARATOR} ready",
        state.model, state.provider
    );
    block.push((centre(&session, width), muted));
    block.push((blank(width), text_style()));
    for (key, outcome) in STARTERS {
        let line = format!("{}{:<8}{outcome}", " ".repeat(starter_indent), key);
        block.push((pad(&line, width), text_style()));
    }

    // Centre the block vertically, biased one row up when the gap is odd, matching
    // the design frame.
    let top = rows.saturating_sub(block.len()).div_ceil(2);
    let mut out: Vec<(String, Style)> = Vec::with_capacity(rows);
    for _ in 0..top {
        out.push((blank(width), text_style()));
    }
    out.append(&mut block);
    while out.len() < rows {
        out.push((blank(width), text_style()));
    }
    out.truncate(rows);
    out
}

/// Centre `text` in `width` columns, floor-biased to the left, matching the design.
fn centre(text: &str, width: usize) -> String {
    let indent = width.saturating_sub(text.width()) / 2;
    pad(&format!("{}{text}", " ".repeat(indent)), width)
}

// ---- The panels. ----------------------------------------------------------

/// The transient panel lines, framed by two full-width rules.
fn panel_lines(state: &TuiState, width: usize) -> Vec<(String, Style)> {
    match &state.panel {
        Panel::None => Vec::new(),
        Panel::Approval(approval) => approval_panel(approval, width),
        Panel::SlashList(list) => slash_panel(list, width),
        Panel::Help => help_panel(width),
    }
}

fn approval_panel(approval: &Approval, width: usize) -> Vec<(String, Style)> {
    let caution = style_for(Role::Caution);
    let rule = (rule_line(width), caution);
    let slot = duration_slot(approval.millis);
    let left = format!(
        "{GLYPH_APPROVAL}  approval {GLYPH_SEPARATOR} {}",
        sanitize_line(&approval.title)
    );
    let right = format!("{slot}  ");
    vec![
        rule.clone(),
        (justify(&left, &right, width), caution),
        (
            pad(&format!("     {}", sanitize_line(&approval.command)), width),
            text_style(),
        ),
        (
            pad(&format!("     {APPROVAL_CHOICES}"), width),
            text_style(),
        ),
        rule,
    ]
}

fn slash_panel(list: &SlashList, width: usize) -> Vec<(String, Style)> {
    let muted = style_for(Role::Muted);
    let mut lines = vec![(rule_line(width), muted)];
    for (index, command) in filter_slash_commands(&list.query).iter().enumerate() {
        let prefix = if index == list.selected {
            format!(" {GLYPH_USER} ")
        } else {
            "   ".to_string()
        };
        let body = format!("{prefix}{:<12}{}", command.name, command.summary);
        let style = if index == list.selected {
            text_style().add_modifier(Modifier::REVERSED)
        } else {
            text_style()
        };
        lines.push((pad(&body, width), style));
    }
    lines.push((rule_line(width), muted));
    lines
}

fn help_panel(width: usize) -> Vec<(String, Style)> {
    let muted = style_for(Role::Muted);
    let mut lines = vec![(rule_line(width), muted)];
    lines.push((pad("  keys", width), text_style()));
    for binding in bindings() {
        // An unwired binding is drawn muted and says so, so the help screen never
        // promises a key that answers nothing. See `D-a-panel-nobody-can-open`.
        let (note, style) = if binding.built {
            ("", text_style())
        } else {
            (" · not built yet", muted)
        };
        lines.push((
            pad(
                &format!("    {:<17}{}{note}", binding.keys, binding.summary),
                width,
            ),
            style,
        ));
    }
    lines.push((pad("  commands", width), text_style()));
    lines.push((pad(&format!("    {HELP_COMMANDS}"), width), muted));
    lines.push((rule_line(width), muted));
    lines
}

// ---- The composer. --------------------------------------------------------

/// The composer box: a rounded frame around the draft or the placeholder.
fn composer_lines(state: &TuiState, width: usize) -> Vec<(String, Style)> {
    let muted = style_for(Role::Muted);
    let inner_width = width.saturating_sub(2);
    let show_placeholder = state.input.is_empty()
        && state.activity == ActivityState::Idle
        && state.panel == Panel::None;

    let mut rows: Vec<String> = Vec::new();
    if show_placeholder {
        rows.push(format!(" {GLYPH_USER} {PLACEHOLDER}"));
    } else {
        let draft = sanitize_line(&state.input);
        rows.push(format!(" {GLYPH_USER} {draft}{GLYPH_CURSOR}"));
    }

    let mut lines = vec![(border(width, '╭', '╮'), muted)];
    for (index, row) in rows.into_iter().enumerate() {
        let inner = pad(&row, inner_width);
        // The placeholder is muted, as `docs/tui-design.md` section 8 states. It drew in
        // the default foreground, which reads as bright as the assistant's answer.
        let style = if show_placeholder && index == 0 {
            muted
        } else {
            text_style()
        };
        lines.push((format!("│{inner}│"), style));
    }
    lines.push((border(width, '╰', '╯'), muted));
    lines
}

// ---- The footer. ----------------------------------------------------------

/// The footer content row, the column where the working word begins, and the column where
/// the key hints begin. The word column is `None` when no word animates. The hint column
/// lets the caller paint the hints muted and the activity word as text.
fn footer_line(state: &TuiState, width: usize) -> (String, Option<usize>, usize) {
    let running =
        state.activity == ActivityState::Running || matches!(state.panel, Panel::Approval(_));
    let (left, word_col) = if running {
        let word = if matches!(state.panel, Panel::Approval(_)) {
            "waiting"
        } else if state.canceling {
            // A cancel the model has not answered yet still shows, because a run that
            // keeps saying "working" after Ctrl-C reads as a dead interface.
            "canceling"
        } else {
            "working"
        };
        let dur = format_duration(state.turn_millis).unwrap_or_default();
        // The word sits after the activity mark and one space.
        let col = EDGE_MARGIN + GLYPH_ACTIVITY.width() + 1;
        (
            format!("{GLYPH_ACTIVITY} {word} {GLYPH_SEPARATOR} {dur}"),
            Some(col),
        )
    } else if state.last_error {
        let dur = format_duration(state.turn_millis).unwrap_or_default();
        (
            format!("done {GLYPH_SEPARATOR} error {GLYPH_SEPARATOR} {dur}"),
            None,
        )
    } else if let Some(reason) = state.last_stop {
        let dur = format_duration(state.turn_millis).unwrap_or_default();
        (
            format!(
                "done {GLYPH_SEPARATOR} {} {GLYPH_SEPARATOR} {dur}",
                stop_word(reason)
            ),
            None,
        )
    } else {
        ("ready".to_string(), None)
    };

    let right = footer_hints(state, width);
    let avail = width.saturating_sub(EDGE_MARGIN * 2);
    let gap = avail
        .saturating_sub(left.width())
        .saturating_sub(right.width());
    let line = format!("  {left}{}{right}  ", " ".repeat(gap));
    // Where the hints start, so the caller can paint them muted.
    let hint_at = EDGE_MARGIN + left.width() + gap;
    (pad(&line, width), word_col, hint_at)
}

/// Repaint the style of the cells from `start` to `end` on row `y`, keeping the symbols.
/// The footer holds two roles on one row, and the sweep uses the same mechanism.
fn restyle(frame: &mut Frame<'_>, y: usize, start: usize, end: usize, style: Style) {
    let buf = frame.buffer_mut();
    let width = buf.area.width as usize;
    for column in start..end.min(width) {
        buf[(column as u16, y as u16)].set_style(style);
    }
}

/// The narrowest width that still shows the full footer hints. Below it, the hints
/// reduce to `? help`, as the design's 40-column tier requires.
const FULL_HINTS_MIN_WIDTH: usize = 80;

fn footer_hints(state: &TuiState, width: usize) -> &'static str {
    // The armed exit gate outranks every other hint. `state.status` held this message
    // for a whole sprint and no code drew it, so a user reported that Ctrl-C does not
    // quit, when it quits 11 ms after the second press.
    if state.exit_armed {
        if width < FULL_HINTS_MIN_WIDTH {
            return "ctrl-c again quits";
        }
        return "press ctrl-c again to quit · any key to stay";
    }
    if width < FULL_HINTS_MIN_WIDTH {
        return "? help";
    }
    match &state.panel {
        Panel::SlashList(_) => "↑ ↓ choose · enter run · esc close",
        Panel::Help => "esc close",
        Panel::Approval(_) => "ctrl-c cancel · / commands · ? help",
        Panel::None => {
            if state.activity == ActivityState::Running {
                "ctrl-c cancel · / commands · ? help"
            } else {
                "enter send · / commands · ? help"
            }
        }
    }
}

/// The footer word for a stop reason. `EndTurn` reads `end turn`, matching the
/// design frame.
fn stop_word(reason: rho_core::AgentStopReason) -> &'static str {
    use rho_core::AgentStopReason::*;
    match reason {
        EndTurn => "end turn",
        MaxTokens => "max tokens",
        MaxTurnRequests => "max turns",
        Refusal => "refusal",
        Canceled => "canceled",
    }
}

/// Apply the motion sweep to the footer working word, writing styles into the
/// cells that already hold it. No `Vec` is allocated, so the sweep costs zero
/// allocations, which is the target the cost budget marks for this stage.
fn apply_sweep(state: &TuiState, frame: &mut Frame<'_>, y: usize, word_at: Option<usize>) {
    let Some(start) = word_at else { return };
    if !state.animate || state.activity != ActivityState::Running {
        return;
    }
    let buf = frame.buffer_mut();
    // The word is the run of non-space cells from `start`.
    let width = buf.area.width as usize;
    for column in start..width {
        let cell = &buf[(column as u16, y as u16)];
        if cell.symbol() == " " {
            break;
        }
        let weight = sweep_weight(state.tick, column - start);
        let style = match motion_cell(weight) {
            MotionCell::Dim => Style::default().add_modifier(Modifier::DIM),
            MotionCell::Plain => Style::default(),
            MotionCell::Bold => Style::default().add_modifier(Modifier::BOLD),
        };
        buf[(column as u16, y as u16)].set_style(style);
    }
}

// ---- Small helpers. -------------------------------------------------------

/// Write one full-width row into the frame buffer, clipped to the width.
fn put(frame: &mut Frame<'_>, y: usize, width: usize, text: &str, style: Style) {
    frame
        .buffer_mut()
        .set_stringn(0, y as u16, text, width, style);
}

/// A run of `width` spaces.
fn blank(width: usize) -> String {
    " ".repeat(width)
}

/// A full-width rule in the box-drawing dash.
fn rule_line(width: usize) -> String {
    "─".repeat(width)
}

/// A composer border row, from the left corner to the right corner.
fn border(width: usize, left: char, right: char) -> String {
    let mid = width.saturating_sub(2);
    format!("{left}{}{right}", "─".repeat(mid))
}

/// Place `left` at the start and `right` at the end of a `width` row, filling the
/// middle with spaces. When the two do not fit, the row is truncated to width.
fn justify(left: &str, right: &str, width: usize) -> String {
    let gap = width
        .saturating_sub(left.width())
        .saturating_sub(right.width());
    pad(&format!("{left}{}{right}", " ".repeat(gap)), width)
}

/// Pad `text` with trailing spaces to exactly `width` display columns, or hard-cut
/// it to width when it is too long. No ellipsis, because the callers size their
/// text to fit.
fn pad(text: &str, width: usize) -> String {
    let have = text.width();
    if have == width {
        text.to_string()
    } else if have < width {
        format!("{text}{}", " ".repeat(width - have))
    } else {
        // Hard-cut to width, honouring display width so a wide glyph is not split.
        let mut out = String::new();
        let mut used = 0usize;
        for ch in text.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w > width {
                break;
            }
            out.push(ch);
            used += w;
        }
        while used < width {
            out.push(' ');
            used += 1;
        }
        out
    }
}

/// Greedy word wrap to `width` display columns. A single space joins words.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current = word.to_string();
        } else if current.width() + 1 + word.width() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// The finished span of a row, if the state records one.
fn row_duration(state: &TuiState, index: usize) -> Option<i64> {
    state.row_durations.get(index).copied().flatten()
}

/// The fold of a tool row. A row the index does not reach is collapsed.
fn row_fold(state: &TuiState, index: usize) -> RowFold {
    state
        .row_folds
        .get(index)
        .copied()
        .unwrap_or(RowFold::Collapsed)
}

/// A one-glyph marker for a tool status, in the UTF-8 tier.
fn status_glyph(status: ToolRowStatus) -> &'static str {
    match status {
        ToolRowStatus::Pending => "·",
        ToolRowStatus::Running => "●",
        ToolRowStatus::Ok => "✓",
        ToolRowStatus::Failed => "✗",
    }
}

/// The plain text style, the terminal default.
fn text_style() -> Style {
    Style::default()
}

/// A ratatui style for a role, in the 256-colour mode.
///
/// `docs/tui-design.md` section 3 gives three columns for every role: a 256-colour value,
/// a 16-colour value, and a no-colour modifier set. They are alternatives, one per
/// terminal mode, and not a stack. This function was adding the no-colour modifiers on
/// top of the 256-colour value, so every muted row drew grey 245 **and** `DIM`. That is
/// two dimmings where the design measured one, and the contrast table in section 4 assumes
/// 245 alone at 5.19 to 1. A user reported the footer as almost invisible.
///
/// `caution` keeps its bold weight, because the design gives the approval panel a stronger
/// weight in every mode. No other role carries a modifier here.
fn style_for(role: Role) -> Style {
    let style = match role_256(role) {
        Some(index) => Style::default().fg(Color::Indexed(index)),
        None => Style::default(),
    };
    if role == Role::Caution {
        return style.add_modifier(Modifier::BOLD);
    }
    style
}
