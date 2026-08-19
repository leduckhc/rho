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
use crate::concise::RowFold;
use crate::duration::{duration_slot, format_duration};
use crate::motion::{MotionCell, motion_cell, sweep_weight};
use crate::sanitize::sanitize_line;
use crate::state::{
    ActivityState, Approval, HistorySearch, Panel, Row, SlashList, ToolRowStatus, TuiState,
    filter_history, row_is_final,
};
use crate::theme::{Role, role_256};

/// The brand mark. The `ρ` renders in the accent role, so it is the first accent
/// on screen.
const BRAND: &str = "ρ rho";
/// The composer placeholder, shown in `muted` when the draft is empty.
const PLACEHOLDER: &str = "Type a prompt. / for commands. ? for help.";
/// The verbatim approval choices row. There is no allow-always choice, because
/// `D-no-remembered-execute-allow` forbids a remembered execute approval.
const APPROVAL_CHOICES: &str = "[y] allow once · [n] deny · [esc] deny and cancel the turn";

/// The widest transcript measure. Text wraps to `min(TEXT_MEASURE_CAP, width - MARGIN)`,
/// so a wide terminal keeps a readable line length.
const TEXT_MEASURE_CAP: usize = 80;
/// The columns the transcript measure leaves off the frame width.
const TEXT_MEASURE_MARGIN: usize = 10;
/// The most history matches the search panel lists at once. The panel borrows its rows
/// from the live area, so it must stay small.
const HISTORY_MATCH_ROWS: usize = 5;

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
/// The quote bar that marks an approval's verbatim text.
const GLYPH_QUOTE: &str = "┃";

/// The band height rho asks for, in rows.
///
/// One footer, a composer of up to ten rows, and at least three live rows.
pub const BAND_ROWS: u16 = 14;

/// The band height for a terminal of `height` rows.
///
/// The band never takes the whole terminal. A short terminal gets `height - 1`. A
/// one-row terminal gets one row.
pub fn band_rows(height: u16) -> u16 {
    match height {
        0 => 0,
        1 => 1,
        h if h < BAND_ROWS + 1 => h - 1,
        _ => BAND_ROWS,
    }
}

/// The band regions that survive at a height, and how many live rows fit.
///
/// The renderer and the click mapping both read this, so the band geometry has one
/// source. It names the live area, the composer, the panel, and the footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    /// The count of panel rows the band draws. Zero means no panel.
    ///
    /// This was a `bool`, and a panel taller than the band was therefore granted nothing.
    /// The help panel wanted twenty-seven rows, so the help screen drew none of them. A
    /// count lets a panel take what fits and window the rest. See
    /// `D-ledger-wins-the-band`.
    pub panel_rows: usize,
    /// True when the transient panel fits at all.
    pub panel: bool,
    /// True when the composer rules fit.
    pub composer_border: bool,
    /// True when at least one composer draft row fits.
    pub composer_input: bool,
    /// The count of composer draft rows the band draws, between the two rules.
    pub composer_rows: usize,
    /// True when the footer fits.
    pub footer: bool,
    /// The count of live rows the band draws.
    pub live_rows: usize,
}

/// One batch of final rows, ready for `Terminal::insert_before`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreezeBatch {
    /// The rendered lines, oldest first. They carry no style, and `sanitize_line` has
    /// already run, because the terminal owns these cells after the insert.
    pub lines: Vec<String>,
    /// The count of transcript rows the lines cover.
    pub rows: usize,
}

/// Draw the band. Pure. No IO. Safe to call every frame.
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
    let (panel_want, panel_floor) = panel_demand(state);

    // Decide the band for this height. The footer keeps its row, a panel takes its rows,
    // the composer scrolls in what remains, and the live rows yield first. See
    // `D-ledger-wins-the-band`.
    let band = plan_band_with_floor(area.height, input_rows, panel_want, panel_floor);
    let panel = panel_lines(state, width, band.panel_rows);

    // Write the surviving regions top to bottom, straight into the buffer.
    let mut y = 0usize;
    let live: Vec<(String, Style)> = if state.rows.is_empty() && state.panel == Panel::None {
        empty_state(state, width, band.live_rows)
    } else {
        live_window_styled(state, area.width, band.live_rows as u16)
    };
    for offset in 0..band.live_rows {
        match live.get(offset) {
            Some((text, style)) => put(frame, y, width, text, *style),
            None => put(frame, y, width, &blank(width), text_style()),
        }
        y += 1;
    }
    if band.panel {
        for (text, style) in &panel {
            put(frame, y, width, text, *style);
            y += 1;
        }
    }
    if band.composer_border {
        let (text, style) = &composer[0];
        put(frame, y, width, text, *style);
        y += 1;
    }
    if band.composer_input {
        // The draft windows to the granted rows, and the last row holds the cursor.
        let draft = &composer[1..composer.len() - 1];
        let start = draft.len().saturating_sub(band.composer_rows);
        for (text, style) in &draft[start..] {
            put(frame, y, width, text, *style);
            y += 1;
        }
    }
    if band.composer_border {
        let (text, style) = &composer[composer.len() - 1];
        put(frame, y, width, text, *style);
        y += 1;
    }
    if band.footer {
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
/// asks the same `plan_band` the renderer uses, which keeps one source of truth for the
/// geometry. It returns `None` when the click misses the list, when no list is open, or
/// when the band is too short to draw the panel.
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
    // The panel is one row for each command, and it draws no rules of its own.
    let band = plan_band(height, input_rows, count);
    if !band.panel {
        return None;
    }
    // The panel sits directly below the live rows, and its first row is the first command.
    let first_command = band.live_rows;
    let clicked = row as usize;
    if clicked >= first_command && clicked < first_command + count {
        Some(clicked - first_command)
    } else {
        None
    }
}

/// Plan the band for one frame area. One source of geometry, for the renderer and for
/// the click mapping.
///
/// The live rows always keep at least one row. Regions are added back in keep-priority
/// order, the reverse of the drop order. A region that cannot fit is skipped, and its
/// rows fall through to the live area.
/// Hand out the band's rows in Ledger's rank.
///
/// The regions want twenty rows and the band holds fourteen, so the rank decides. The
/// footer keeps its row. A panel takes its rows next, and `panel_floor` is the count it
/// never yields. The composer scrolls inside what remains and keeps three rows, which is
/// one draft row between two rules. The live rows yield first. See
/// `D-ledger-wins-the-band`.
///
/// `panel_floor` is what makes an approval safe. An approval states a destructive command,
/// so it states the whole of it, including the root the command runs in. An earlier band
/// dropped that row to fit a tall draft.
pub fn plan_band(height: u16, input_rows: usize, panel_want: usize) -> Band {
    plan_band_with_floor(height, input_rows, panel_want, 0)
}

/// `plan_band`, with a count of panel rows that never yield.
pub fn plan_band_with_floor(
    height: u16,
    input_rows: usize,
    panel_want: usize,
    panel_floor: usize,
) -> Band {
    const COMPOSER_RULES: usize = 2;

    let mut left = height as usize;

    // 0. The transcript is never erased. It is the reason the band exists, so one live row
    //    outranks every piece of chrome. A one-row band is all transcript and no chrome.
    let live_floor = 1.min(left);
    left -= live_floor;

    // 1. The footer keeps its row. It carries the activity word, so a band without it
    //    cannot say whether rho works or waits.
    let footer = left >= 1;
    left -= usize::from(footer);

    // The composer keeps three rows where it can: one draft row between two rules.
    let composer_want = input_rows + COMPOSER_RULES;
    let composer_floor = composer_want.min(COMPOSER_RULES + 1).min(left);

    // 2. The panel takes its rows, and it leaves the composer's three standing. A floor
    //    outranks the composer's extra rows, because an approval yields nothing.
    let polite_cap = left.saturating_sub(composer_floor);
    let mut panel_rows = panel_want.min(polite_cap);
    let demanded = panel_floor.min(panel_want);
    if panel_rows < demanded {
        panel_rows = demanded.min(left.saturating_sub(composer_floor));
    }
    left -= panel_rows;

    // 3. The composer scrolls inside what is left. It degrades in one stated order: the
    //    full draft, then a windowed draft between its rules, then a bare draft row with
    //    the rules dropped. A row that would draw nothing is never reserved.
    let (composer_border, composer_rows) = if left >= composer_want {
        (true, input_rows)
    } else if left > COMPOSER_RULES {
        (true, left - COMPOSER_RULES)
    } else if left >= 1 && input_rows > 0 {
        (false, left)
    } else {
        (false, 0)
    };
    left -= composer_rows + if composer_border { COMPOSER_RULES } else { 0 };

    Band {
        panel_rows,
        panel: panel_rows > 0,
        composer_border,
        composer_input: composer_rows > 0,
        composer_rows,
        footer,
        live_rows: live_floor + left,
    }
}

// ---- The banner. ----------------------------------------------------------

/// The one-line banner, frozen above the band when the session starts.
///
/// It holds the brand, the working directory, the branch, and the model. These do not
/// change during a session, so the banner freezes once.
pub fn banner_line(state: &TuiState, width: usize) -> String {
    // An empty field draws no separator. The banner once read `ρ rho   ·  · model ·`,
    // because the renderer joined four fields and nothing filled three of them.
    let parts = [
        state.cwd.as_str(),
        state.branch.as_str(),
        state.model.as_str(),
        state.provider.as_str(),
    ];
    let joined = parts
        .iter()
        .filter(|part| !part.trim().is_empty())
        .copied()
        .collect::<Vec<&str>>()
        .join(&format!(" {GLYPH_SEPARATOR} "));
    let text = if joined.is_empty() {
        BRAND.to_string()
    } else {
        format!("{BRAND}  {joined}")
    };
    pad(&text, width)
}

/// The banner batch to freeze at startup, or `None` when it is already frozen.
///
/// The loop holds the `frozen` flag and calls this once. So the scrollback holds one
/// banner, and a second startup step inserts no second banner. The batch covers no
/// transcript row, so it never advances `frozen_rows`.
pub fn banner_freeze(state: &TuiState, width: u16, frozen: bool) -> Option<FreezeBatch> {
    if frozen {
        return None;
    }
    Some(FreezeBatch {
        lines: vec![banner_line(state, width as usize)],
        rows: 0,
    })
}

// ---- The live area and the freeze. ----------------------------------------

/// The live rows as text, newest-anchored, one entry per drawn row.
///
/// It keeps the newest lines when the live rows do not fit. A line that scrolls out of
/// the live area reaches the scrollback when its row freezes.
pub fn live_window(state: &TuiState, width: u16, rows: u16) -> Vec<String> {
    live_window_styled(state, width, rows)
        .into_iter()
        .map(|(line, _style)| line)
        .collect()
}

/// The live rows as styled lines, newest-anchored, one entry per drawn row.
///
/// This keeps the per-row style, so a reasoning row draws dimmed in the live band. The
/// public `live_window` drops the style, because a frozen row reaches the scrollback as
/// plain text that the terminal then owns.
fn live_window_styled(state: &TuiState, width: u16, rows: u16) -> Vec<(String, Style)> {
    let rows = rows as usize;
    if rows == 0 {
        return Vec::new();
    }
    let start = state.frozen_rows.min(state.rows.len());
    let mut lines = render_rows_styled(state, start, state.rows.len(), width as usize);
    // Drop any overflow from the top, so the newest row stays visible.
    if lines.len() > rows {
        lines.drain(0..lines.len() - rows);
    }
    lines
}

/// The next batch to freeze, or `None` when the oldest unfrozen row is still live.
///
/// It takes the longest final prefix, so the scrollback keeps the session order. A row
/// behind a live row waits, even when the row itself is final.
pub fn next_freeze(state: &TuiState, width: u16) -> Option<FreezeBatch> {
    let start = state.frozen_rows.min(state.rows.len());
    let mut stop = start;
    while stop < state.rows.len() && row_is_final(state, stop) {
        stop += 1;
    }
    if stop == start {
        return None;
    }
    let lines = render_rows_plain(state, start, stop, width as usize);
    Some(FreezeBatch {
        lines,
        rows: stop - start,
    })
}

/// Every remaining row, whatever its state, for the exit path only.
///
/// After the loop leaves, no event can arrive, so a row that never finished will never
/// change again. It freezes as it stands: a running tool row reads `running`, which is the
/// truth of a cancelled session. The running loop must use `next_freeze` instead, because a
/// task and a child outlive their turn. See `SPEC-tui-inline-and-composer` section 3.6.
pub fn freeze_all(state: &TuiState, width: u16) -> Option<FreezeBatch> {
    let start = state.frozen_rows.min(state.rows.len());
    let stop = state.rows.len();
    if stop == start {
        return None;
    }
    let lines = render_rows_plain(state, start, stop, width as usize);
    Some(FreezeBatch {
        lines,
        rows: stop - start,
    })
}

/// Render the rows from `start` to `stop` as plain text, oldest first.
///
/// A blank row separates turns, and it leads the row below it. A run of tool rows stays
/// together. So the last line is always a content line, never a blank separator.
fn render_rows_plain(state: &TuiState, start: usize, stop: usize, width: usize) -> Vec<String> {
    render_rows_styled(state, start, stop, width)
        .into_iter()
        .map(|(text, _style)| text)
        .collect()
}

/// Render the rows from `start` to `stop` as styled lines, oldest first.
///
/// The style stays with each line, so the live band can draw a reasoning row dimmed. The
/// freeze path drops the style, because the terminal owns the scrollback cells.
fn render_rows_styled(
    state: &TuiState,
    start: usize,
    stop: usize,
    width: usize,
) -> Vec<(String, Style)> {
    let measure = TEXT_MEASURE_CAP.min(width.saturating_sub(TEXT_MEASURE_MARGIN));
    let mut out: Vec<(String, Style)> = Vec::new();
    for index in start..stop {
        let row = &state.rows[index];
        let is_tool = matches!(row, Row::Tool { .. });
        let prev_tool = index > 0 && matches!(state.rows[index - 1], Row::Tool { .. });
        if !(is_tool && prev_tool) {
            out.push((blank(width), text_style()));
        }
        push_row(&mut out, state, index, row, width, measure);
    }
    out
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
        Row::Thinking { text } => {
            // Reasoning draws in `Role::Muted`, never `Role::Text`, because it is the
            // model's private work and must not read as the answer. The mode decides how
            // much shows. See `SPEC-reasoning-across-providers` section 5.
            let summary = match format_duration(row_duration(state, index)) {
                Some(span) => format!("{GLYPH_THINKING} thought for {span}"),
                None => format!("{GLYPH_THINKING} thinking"),
            };
            match state.reasoning_display {
                rho_core::ReasoningDisplay::Off => {}
                rho_core::ReasoningDisplay::Summary => {
                    out.push((pad(&summary, width), style_for(Role::Muted)));
                }
                rho_core::ReasoningDisplay::Full => {
                    out.push((pad(&summary, width), style_for(Role::Muted)));
                    for line in wrap(&sanitize_line(text), measure) {
                        out.push((pad(&line, width), style_for(Role::Muted)));
                    }
                }
                rho_core::ReasoningDisplay::Live => {
                    // The span settles when the answer starts, so a settled span means the
                    // reasoning collapses to the summary row. While it streams, the text
                    // shows.
                    if row_duration(state, index).is_some() {
                        out.push((pad(&summary, width), style_for(Role::Muted)));
                    } else {
                        for line in wrap(&sanitize_line(text), measure) {
                            out.push((pad(&line, width), style_for(Role::Muted)));
                        }
                    }
                }
            }
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

/// Ledger's tool row: two columns of indent, the status glyph, the verb, the payload, and
/// the duration flush right.
///
/// The status leads because the eye scans a left column and hunts a right one. The row
/// carried its glyph last, after the duration, so a column of glyphs never formed. The
/// indent separates a tool row from prose without spending a colour. See
/// `D-ledger-wins-the-band`.
fn tool_header(
    state: &TuiState,
    index: usize,
    name: &str,
    payload: &str,
    status: ToolRowStatus,
    width: usize,
) -> String {
    // No fold caret. A caret promises `ctrl-o`, and no key folds a row in this build. A
    // promise on screen that no key answers is `D-a-panel-nobody-can-open`. The caret
    // comes back with the fold keys, and `fold_caret` stays for that stage.
    let left = format!(
        "  {} {}  {}",
        status_glyph(status),
        name,
        sanitize_line(payload)
    );
    let right = duration_slot(row_duration(state, index));
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
/// The rows a panel wants, and the rows it never yields.
///
/// The floor is what keeps an approval whole. See `D-ledger-wins-the-band`.
fn panel_demand(state: &TuiState) -> (usize, usize) {
    match &state.panel {
        Panel::None => (0, 0),
        // An approval yields nothing. Every row of it is safety information.
        Panel::Approval(_) => (APPROVAL_ROWS, APPROVAL_ROWS),
        // The help window wants the whole table, and it settles for a window.
        Panel::Help => (1 + bindings().len(), HELP_MIN_ROWS),
        // The counts below must equal the rows the panel draws. They did not, and the
        // truncation in `panel_lines` then ate the last command of the palette.
        Panel::SlashList(list) => (filter_slash_commands(&list.query).len(), 1),
        Panel::HistorySearch(search) => {
            let matches = filter_history(&state.history, &search.query).len();
            // The matches, or one row saying there were none, plus the query row.
            (matches.clamp(1, HISTORY_MATCH_ROWS) + 1, 2)
        }
    }
}

/// The approval panel is four rows: the request, the command, the root, and the choices.
const APPROVAL_ROWS: usize = 4;
/// The help window never shrinks below a header and two keys.
const HELP_MIN_ROWS: usize = 3;

fn panel_lines(state: &TuiState, width: usize, budget: usize) -> Vec<(String, Style)> {
    if budget == 0 {
        return Vec::new();
    }
    let mut lines = match &state.panel {
        Panel::None => Vec::new(),
        Panel::Approval(approval) => approval_panel(approval, width),
        Panel::SlashList(list) => slash_panel(list, width),
        Panel::Help => help_panel(width, budget, state.help_offset),
        Panel::HistorySearch(search) => history_search_panel(search, &state.history, width),
    };
    // A panel never overruns its grant. It windowed itself where it could, and this is the
    // backstop that keeps the band's arithmetic honest.
    lines.truncate(budget);
    lines
}

/// The reverse-search panel: the query row, then the matches, newest first.
///
/// It frames the matches with two rules, like the slash list. It marks the selected
/// match, so the user sees which entry `enter` accepts. The panel once drew the query row
/// alone, while this comment already claimed the rest, so a search showed no result.
fn history_search_panel(
    search: &HistorySearch,
    history: &[String],
    width: usize,
) -> Vec<(String, Style)> {
    let muted = style_for(Role::Muted);
    let mut lines = Vec::new();
    let matches = filter_history(history, &search.query);
    for (row, index) in matches.iter().take(HISTORY_MATCH_ROWS).enumerate() {
        let entry = history.get(*index).map(String::as_str).unwrap_or_default();
        let prefix = if row == search.selected {
            format!(" {GLYPH_USER} ")
        } else {
            "   ".to_string()
        };
        let body = format!("{prefix}{}", sanitize_line(entry));
        let style = if row == search.selected {
            text_style().add_modifier(Modifier::REVERSED)
        } else {
            text_style()
        };
        lines.push((pad(&body, width), style));
    }
    if matches.is_empty() {
        // A search that found nothing must say so, because an empty panel reads as a
        // defect. This is the house rule from `D-a-panel-nobody-can-open`.
        lines.push((pad("   no match in this session", width), muted));
    }
    lines.push((
        pad(
            &format!(
                "  search {GLYPH_SEPARATOR} {}",
                sanitize_line(&search.query)
            ),
            width,
        ),
        text_style(),
    ));
    lines
}

/// The approval panel: the request, the command, the root, and the choices.
///
/// Four rows, and it yields none of them. A command means nothing without the tree it acts
/// on, so the root is not decoration. See `D-ledger-wins-the-band`.
fn approval_panel(approval: &Approval, width: usize) -> Vec<(String, Style)> {
    let caution = style_for(Role::Caution);
    let muted = style_for(Role::Muted);
    let slot = duration_slot(approval.millis);
    let left = format!("  {GLYPH_APPROVAL} {}", sanitize_line(&approval.title));
    vec![
        (justify(&left, &slot, width), caution),
        (
            pad(
                &format!("  {GLYPH_QUOTE} {}", sanitize_line(&approval.command)),
                width,
            ),
            text_style(),
        ),
        (
            pad(
                &format!(
                    "  {GLYPH_QUOTE}  the session root is {}",
                    sanitize_line(&approval.root)
                ),
                width,
            ),
            muted,
        ),
        (pad(&format!("  {APPROVAL_CHOICES}"), width), caution),
    ]
}

fn slash_panel(list: &SlashList, width: usize) -> Vec<(String, Style)> {
    // No rules of its own. The composer's top rule already separates the panel from the
    // draft, and a second rule spent a row of a fourteen-row band on nothing.
    let mut lines = Vec::new();
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
    lines
}

/// The help window: a counted header, then as many keys as the band granted.
///
/// The table is longer than the band. This panel once drew every row, so `plan_band`
/// granted it nothing and the help screen showed **no keys at all**, while a frame fixture
/// pinned that blank output as correct. Now it windows, and it states its own position.
///
/// The total comes from `bindings()`, never from a literal, so adding or removing a
/// binding can never make the header lie.
/// The count of help key rows a band of `height` rows can draw.
///
/// The reducer clamps a scroll key with this, and `help_panel` draws with it. One function
/// serves both, because two copies of a geometry rule drift, and a drifted clamp banks key
/// presses that the screen never answers.
pub fn help_visible_rows(height: u16) -> usize {
    // The help opens over an empty draft, so the composer asks for its one row.
    let band = plan_band_with_floor(height, 1, 1 + bindings().len(), HELP_MIN_ROWS);
    // One granted row pays for the counted header.
    band.panel_rows.saturating_sub(1)
}

fn help_panel(width: usize, budget: usize, offset: usize) -> Vec<(String, Style)> {
    let muted = style_for(Role::Muted);
    let table = bindings();
    let total = table.len();
    // One row of the grant pays for the header, and the keys take the rest.
    let keys = budget.saturating_sub(1).min(total);
    if keys == 0 {
        return Vec::new();
    }
    // The window stops at the last row. A list that scrolls past its end draws blank rows,
    // and a blank row reads as a defect.
    let first = offset.min(total - keys);
    let last = first + keys;
    let header_left = format!("  keys  {}-{} of {total}", first + 1, last);
    let above = first;
    let below = total - last;
    let header_right = match (above, below) {
        (0, 0) => String::new(),
        (0, n) => format!("↓ {n} more below"),
        (n, 0) => format!("↑ {n} more above"),
        (up, down) => format!("↑ {up} · ↓ {down}"),
    };
    let mut lines = vec![(justify(&header_left, &header_right, width), muted)];
    for binding in table.iter().skip(first).take(keys) {
        // An unwired binding says so, so the help never promises a key that answers
        // nothing. See `D-a-panel-nobody-can-open`.
        let (note, style) = if binding.built {
            ("", text_style())
        } else {
            (" · not built yet", muted)
        };
        lines.push((
            pad(
                &format!("  {:<15}{}{note}", binding.keys, binding.summary),
                width,
            ),
            style,
        ));
    }
    lines
}

// ---- The composer. --------------------------------------------------------

/// The composer box: a rounded frame around the draft or the placeholder.
///
/// It draws `display_lines`, so a tall draft wraps and scrolls inside the box. It places
/// the cursor glyph at `cursor_cell`, so the cursor follows the edit and the wrap.
fn composer_lines(state: &TuiState, width: usize) -> Vec<(String, Style)> {
    let muted = style_for(Role::Muted);
    // The sides are open, so the draft owns the whole width. A vertical border has to land
    // on an exact column on every row, and it drifts when a wide glyph misreports its
    // width. A full-width rule cannot drift. See `docs/tui-design.md` section 8.
    let prefix = format!("{GLYPH_USER} ");
    let prefix_width = prefix.width();
    let text_width = width.saturating_sub(prefix_width);
    let show_placeholder = state.draft_is_empty()
        && state.activity == ActivityState::Idle
        && state.panel == Panel::None;

    let mut rows: Vec<String> = Vec::new();
    if show_placeholder {
        rows.push(format!("{prefix}{PLACEHOLDER}"));
    } else {
        let display = state.draft.display_lines(text_width);
        let (cursor_row, cursor_col) = state.draft.cursor_cell(text_width);
        for (index, line) in display.iter().enumerate() {
            let clean = sanitize_line(line);
            let body = if index == cursor_row {
                insert_cursor(&clean, cursor_col)
            } else {
                clean
            };
            let indent = if index == 0 {
                prefix.clone()
            } else {
                " ".repeat(prefix_width)
            };
            rows.push(format!("{indent}{body}"));
        }
    }

    let mut lines = vec![(rule_line(width), muted)];
    for (index, row) in rows.into_iter().enumerate() {
        // The placeholder is muted, as `docs/tui-design.md` section 8 states. It drew in
        // the default foreground, which reads as bright as the assistant's answer.
        let style = if show_placeholder && index == 0 {
            muted
        } else {
            text_style()
        };
        lines.push((pad(&row, width), style));
    }
    lines.push((rule_line(width), muted));
    lines
}

/// Insert the cursor glyph at display column `column` of `text`.
///
/// It walks the display width, so a wide glyph keeps the cursor in line. The glyph lands
/// after the last column when the cursor sits at the row end.
fn insert_cursor(text: &str, column: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    let mut placed = false;
    for ch in text.chars() {
        if !placed && col >= column {
            out.push_str(GLYPH_CURSOR);
            placed = true;
        }
        out.push(ch);
        col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    if !placed {
        out.push_str(GLYPH_CURSOR);
    }
    out
}

/// The wrap width the composer draws its draft at, for a band of `width` columns.
///
/// The loop passes it to the reducer, so a row-motion key wraps the same way the screen
/// does. It is the inner box width less the composer prefix.
pub fn composer_text_width(width: u16) -> usize {
    let inner = (width as usize).saturating_sub(2);
    let prefix_width = format!(" {GLYPH_USER} ").width();
    inner.saturating_sub(prefix_width)
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
    let area = frame.area();
    let row = area.y + y as u16;
    let origin = area.x as usize;
    let width = area.width as usize;
    let buf = frame.buffer_mut();
    for column in start..end.min(width) {
        buf[((origin + column) as u16, row)].set_style(style);
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
        Panel::Help => "↑ ↓ scroll · esc close · / lists commands",
        Panel::HistorySearch(_) => "type to search · enter accept · esc close",
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
    let area = frame.area();
    let row = area.y + y as u16;
    let origin = area.x as usize;
    let width = area.width as usize;
    let buf = frame.buffer_mut();
    // The word is the run of non-space cells from `start`.
    for column in start..width {
        let cell = &buf[((origin + column) as u16, row)];
        if cell.symbol() == " " {
            break;
        }
        let weight = sweep_weight(state.tick, column - start);
        let style = match motion_cell(weight) {
            MotionCell::Dim => Style::default().add_modifier(Modifier::DIM),
            MotionCell::Plain => Style::default(),
            MotionCell::Bold => Style::default().add_modifier(Modifier::BOLD),
        };
        buf[((origin + column) as u16, row)].set_style(style);
    }
}

// ---- Small helpers. -------------------------------------------------------

/// Write one full-width row into the frame buffer, clipped to the width.
fn put(frame: &mut Frame<'_>, y: usize, width: usize, text: &str, style: Style) {
    // The row `y` counts from the top of the band, and the band is not the screen. An
    // inline viewport anchors to the cursor row, so `Frame::area()` carries the origin.
    // A write at an absolute (0, 0) panics there, and every fullscreen fixture missed it.
    let area = frame.area();
    let (x, row) = (area.x, area.y + y as u16);
    frame.buffer_mut().set_stringn(x, row, text, width, style);
}

/// A run of `width` spaces.
fn blank(width: usize) -> String {
    " ".repeat(width)
}

/// A full-width rule in the box-drawing dash.
fn rule_line(width: usize) -> String {
    "─".repeat(width)
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
