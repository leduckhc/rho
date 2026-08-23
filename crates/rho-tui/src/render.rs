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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::bindings::{bindings, filter_slash_commands};
use crate::concise::RowFold;
use crate::duration::{duration_slot, format_duration};
use crate::markdown::{MarkdownKind, has_inline_markup, scan_inline, scan_markdown};
use crate::motion::{MotionCell, motion_cell, sweep_weight};
use crate::sanitize::{sanitize_block, sanitize_line};
use crate::state::{
    ActivityState, Approval, HistorySearch, Panel, Row, SlashList, ToolRowStatus, TuiState,
    filter_history,
};
use crate::styled::{StyledLine, one};
use crate::theme::{Role, role_16, role_256, role_bg_256};
use rho_core::ReasoningDisplay;

/// The brand mark. The `ρ` renders in the accent role, so it is the first accent
/// on screen.
const BRAND: &str = "ρ rho";
/// The composer placeholder, shown in `muted` when the draft is empty.
const PLACEHOLDER: &str = "Type a prompt. / for commands. ? for help.";
/// The verbatim approval choices row. There is no allow-always choice, because
/// `D-no-remembered-execute-allow` forbids a remembered execute approval.
const APPROVAL_CHOICES: &str = "[y] allow once · [n] deny · [esc] deny and cancel the turn";

/// The columns the transcript measure leaves off the frame width.
///
/// Exactly one, and it belongs to the scroll rail, which draws at `width - 1`. Text that used the
/// whole width lost its last character to the rail every time the transcript overflowed.
///
/// Reserving the column only while the rail shows does not work: the measure decides how many rows
/// the text wraps to, the row count decides whether it overflows, and the overflow would decide
/// the measure. So it is reserved always. See `D-text-fills-the-width`.
const RAIL_COLUMN: usize = 1;
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
/// A notice is not a failure, so it must not borrow the error glyph.
const GLYPH_NOTICE: &str = "!";
/// The narrowest text column a notice label may leave behind it.
///
/// Below it the label takes its own row, and the text takes the whole measure. The label
/// used to keep its column at every width, so at width 24 or less the text column reached
/// zero and the whole message vanished. Found by review, then measured.
const NOTICE_MIN_TEXT: usize = 12;
const GLYPH_SEPARATOR: &str = "·";
/// The glyph a horizontal rule repeats.
const GLYPH_RULE: &str = "─";
const GLYPH_CURSOR: &str = "█";
/// The quote bar that marks an approval's verbatim text.
const GLYPH_QUOTE: &str = "┃";

/// The rows the footer always keeps.
const FOOTER_ROWS: u16 = 1;
/// The composer's smallest healthy height: two rules around one draft row.
const COMPOSER_MIN_ROWS: u16 = 3;
/// The most draft rows the composer shows. From `D-ledger-wins-the-band`.
const MAX_DRAFT_ROWS: usize = 10;
/// The rules above and below the draft.
const COMPOSER_RULES: usize = 2;

/// The terminal height rho needs at startup: the composer's draft row and the footer.
///
/// Below it, startup is fatal and returns `TuiError::TooSmall`. A resize below it draws
/// what fits and never ends the session. See `SPEC-tui-alternate-screen` section 7.
pub const STARTUP_MIN_ROWS: u16 = COMPOSER_MIN_ROWS + FOOTER_ROWS;

/// The screen regions and their heights, top to bottom. rho owns the whole terminal now,
/// so there is no band budget. The transcript takes the rows the chrome leaves. See
/// `SPEC-tui-alternate-screen` section 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenLayout {
    /// True when the one-row banner draws at the top. The banner was frozen above the
    /// inline band. It now lives at the top row, the calm edge farthest from the
    /// composer, so the session context stays visible above the transcript. It is dropped
    /// first when the screen is too small. See `SPEC-tui-alternate-screen` section 6b.
    pub banner: bool,
    /// The transcript window height, in display rows. The transcript scrolls inside it.
    pub transcript_rows: usize,
    /// The panel rows drawn below the transcript. Zero means no panel.
    pub panel_rows: usize,
    /// True when the composer draws its two rules.
    pub composer_border: bool,
    /// The composer draft rows drawn, between the rules where they draw.
    pub composer_rows: usize,
    /// True when the footer draws.
    pub footer: bool,
    /// True when the terminal is too small to draw the composer and the footer whole.
    ///
    /// It draws what fits and never a panel. It is fatal only at startup. See section 7.
    pub too_small: bool,
}

/// Plan the full-screen layout for one frame.
///
/// The footer keeps one row. The composer keeps two rules around one draft row at least,
/// because the draft is the one thing the user owns. A panel keeps its `panel_floor` rows
/// next: an approval states a destructive command, so it never yields its session root,
/// even under a tall draft. See `D-ledger-wins-the-band`. The composer then grows to its
/// full draft, then the banner, then the panel to its full want, then the transcript takes
/// the rest.
///
/// While the terminal is too small to draw the composer and the footer whole, it draws
/// what fits in this order: the composer's draft row, then the footer, then the
/// transcript. It draws no panel. See section 7.
pub fn plan_screen(
    height: u16,
    draft_rows: usize,
    panel_want: usize,
    panel_floor: usize,
) -> ScreenLayout {
    let h = height as usize;
    if h == 0 {
        return ScreenLayout {
            banner: false,
            transcript_rows: 0,
            panel_rows: 0,
            composer_border: false,
            composer_rows: 0,
            footer: false,
            too_small: true,
        };
    }
    if height < STARTUP_MIN_ROWS {
        // Too small. Draw the draft row, then the footer, then the transcript. No banner,
        // no panel, no rules. The order is the keep priority, not the screen position.
        let mut left = h;
        let composer_rows = 1.min(left);
        left -= composer_rows;
        let footer = left >= 1;
        left -= usize::from(footer);
        return ScreenLayout {
            banner: false,
            transcript_rows: left,
            panel_rows: 0,
            composer_border: false,
            composer_rows,
            footer,
            too_small: true,
        };
    }
    // Healthy. Reserve the footer, then the composer minimum, then the panel floor. Then
    // grow the composer, add the banner, grow the panel, and give the rest to the
    // transcript.
    let left = h - FOOTER_ROWS as usize;
    let composer_min = COMPOSER_MIN_ROWS as usize;
    let floor = panel_floor
        .min(panel_want)
        .min(left.saturating_sub(composer_min));
    let mut pool = left - composer_min - floor;
    let draft = draft_rows.clamp(1, MAX_DRAFT_ROWS);
    let composer_extra = (draft + COMPOSER_RULES - composer_min).min(pool);
    pool -= composer_extra;
    let composer_total = composer_min + composer_extra;
    let composer_rows = composer_total - COMPOSER_RULES;
    let banner = pool >= 1;
    pool -= usize::from(banner);
    let panel_extra = panel_want.saturating_sub(floor).min(pool);
    pool -= panel_extra;
    let panel_rows = floor + panel_extra;
    let transcript_rows = pool;
    ScreenLayout {
        banner,
        transcript_rows,
        panel_rows,
        composer_border: true,
        composer_rows,
        footer: true,
        too_small: false,
    }
}

/// Draw the whole screen. Pure. No IO. Safe to call every frame.
pub fn render(state: &TuiState, frame: &mut Frame<'_>) {
    let area = frame.area();
    let width = area.width as usize;
    let height = area.height as usize;
    if width == 0 || height == 0 {
        return;
    }

    let composer = composer_lines(state, width);
    let input_rows = composer.len().saturating_sub(2);
    // A too-small screen draws no panel, so it asks for none.
    let (panel_want, panel_floor) = panel_demand(state);
    let layout = plan_screen(area.height, input_rows, panel_want, panel_floor);
    let (panel_want, panel_floor) = if layout.too_small {
        (0, 0)
    } else {
        (panel_want, panel_floor)
    };
    let layout = plan_screen(area.height, input_rows, panel_want, panel_floor);

    let mut y = 0usize;
    // The banner is one row at the top, when the screen has room for it.
    if layout.banner {
        put(
            frame,
            y,
            width,
            &one((banner_line(state, width), style_for(Role::Muted))),
        );
        y += 1;
    }
    // The transcript, windowed by the scroll state and anchored to the newest row.
    let visible = layout.transcript_rows;
    let (lines, total) = transcript_window(state, width, visible);
    let transcript_top = y;
    for line in &lines {
        put(frame, y, width, line);
        y += 1;
    }
    // The rail is one muted column, drawn only when the transcript overflows. It has no
    // arrows, and its shape is out of scope. It overlays the last column of the
    // transcript, so it takes no layout row. See section 6 and `Scroll::hidden`.
    if total > visible && visible > 0 {
        draw_rail(state, frame, transcript_top, visible, total, width);
    }

    // The panel, sized to its content.
    if layout.panel_rows > 0 {
        let panel = panel_lines(state, width, layout.panel_rows);
        for line in &panel {
            put(frame, y, width, line);
            y += 1;
        }
    }
    if layout.composer_border {
        put(frame, y, width, &composer[0]);
        y += 1;
    }
    if layout.composer_rows > 0 {
        // The draft windows to the granted rows, and the last row holds the cursor.
        let draft = &composer[1..composer.len() - 1];
        let start = draft.len().saturating_sub(layout.composer_rows);
        for line in &draft[start..] {
            put(frame, y, width, line);
            y += 1;
        }
    }
    if layout.composer_border {
        put(frame, y, width, &composer[composer.len() - 1]);
        y += 1;
    }
    if layout.footer {
        let (footer, word_at, hint_at) = footer_line(state, width);
        put(frame, y, width, &one((footer.clone(), text_style())));
        restyle(frame, y, hint_at, width, style_for(Role::Muted));
        apply_sweep(state, frame, y, word_at);
    }
}

/// The transcript display lines for the window, padded to `visible`, plus the total.
///
/// The newest row sits at the bottom, directly above the composer, so a transcript that
/// underflows the window pads above. A transcript that overflows windows by the scroll
/// offset. The empty state fills the window when there is nothing to show.
fn transcript_window(state: &TuiState, width: usize, visible: usize) -> (Vec<StyledLine>, usize) {
    if visible == 0 {
        return (Vec::new(), 0);
    }
    if state.panel == Panel::None
        && conversation_is_empty(state)
        && let Some(block) = empty_state(state, width, visible)
    {
        return (block, visible);
    }
    let lines = transcript_lines(state, width);
    let total = lines.len();
    if total <= visible {
        let mut window: Vec<StyledLine> = Vec::with_capacity(visible);
        for _ in 0..(visible - total) {
            window.push(one((blank(width), text_style())));
        }
        window.extend(lines);
        return (window, total);
    }
    let first = state.transcript_offset(total, visible).min(total - visible);
    let window = lines[first..first + visible].to_vec();
    (window, total)
}

/// Every transcript row rendered to display lines, oldest first, with styles kept.
///
/// A blank row separates turns and leads the row below it. A run of tool rows stays
/// together, so the last line is a content line and never a separator.
fn transcript_lines(state: &TuiState, width: usize) -> Vec<StyledLine> {
    let measure = width.saturating_sub(RAIL_COLUMN).max(1);
    let mut out: Vec<StyledLine> = Vec::new();
    for index in 0..state.rows.len() {
        let row = &state.rows[index];
        let is_tool = matches!(row, Row::Tool { .. });
        let prev_tool = index > 0 && matches!(state.rows[index - 1], Row::Tool { .. });
        if !(is_tool && prev_tool) {
            out.push(one((blank(width), text_style())));
        }
        push_row(&mut out, state, index, row, width, measure);
    }
    out
}

/// The transcript metrics for a frame: the total display rows and the window height.
///
/// The app loop calls it each frame and writes the pair into the state, so the reducer
/// clamps a scroll key against the same geometry the renderer draws. One source, so the
/// clamp and the draw cannot drift. See `D-scroll-keys-yield-to-an-empty-draft`.
pub fn transcript_metrics(state: &TuiState, width: u16, height: u16) -> (usize, usize) {
    let width = width as usize;
    if width == 0 {
        return (0, 0);
    }
    let composer = composer_lines(state, width);
    let input_rows = composer.len().saturating_sub(2);
    let (panel_want, panel_floor) = panel_demand(state);
    let layout = plan_screen(height, input_rows, panel_want, panel_floor);
    let (panel_want, panel_floor) = if layout.too_small {
        (0, 0)
    } else {
        (panel_want, panel_floor)
    };
    let layout = plan_screen(height, input_rows, panel_want, panel_floor);
    let visible = layout.transcript_rows;
    if visible == 0 {
        return (0, visible);
    }
    // The empty state fills the window itself, so it reports no scrollable rows. The
    // condition must match `transcript_window` exactly, or a scroll key would clamp
    // against geometry the renderer never drew.
    if state.panel == Panel::None
        && conversation_is_empty(state)
        && empty_state(state, width, visible).is_some()
    {
        return (0, visible);
    }
    let total = transcript_lines(state, width).len();
    (total, visible)
}

/// Draw the scroll rail in the last column, muted, with a plain thumb for the view.
fn draw_rail(
    state: &TuiState,
    frame: &mut Frame<'_>,
    top: usize,
    visible: usize,
    total: usize,
    width: usize,
) {
    let (above, _below) = state.scroll_hidden();
    let col = width.saturating_sub(1);
    let thumb = ((visible * visible) / total).max(1).min(visible);
    let travel = visible - thumb;
    let hidden = total.saturating_sub(visible).max(1);
    let thumb_top = (above * travel) / hidden;
    for row in 0..visible {
        let style = if row >= thumb_top && row < thumb_top + thumb {
            text_style()
        } else {
            style_for(Role::Muted)
        };
        put_char(frame, top + row, col, "\u{2502}", style);
    }
}

/// The slash-command index drawn at screen row `row`, if any.
///
/// A mouse click carries a row, and the row means nothing without the layout. So this asks
/// the same `plan_screen` the renderer uses, which keeps one source of truth. It returns
/// `None` when the click misses the list, when no list is open, or when the screen is too
/// short to draw the panel.
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
    let layout = plan_screen(height, input_rows, count, 0);
    if layout.panel_rows == 0 {
        return None;
    }
    // The panel sits directly below the transcript, and its first row is the first command.
    // The banner takes the top row when it draws, so the panel starts one row lower. Missing
    // that row made every click land on the wrong command.
    let first_command = usize::from(layout.banner) + layout.transcript_rows;
    let clicked = row as usize;
    if clicked >= first_command && clicked < first_command + count {
        Some(clicked - first_command)
    } else {
        None
    }
}

// ---- The banner. ----------------------------------------------------------

/// The one-line banner. It holds the brand, the working directory, the branch, and the
/// model, which do not change during a session.
pub fn banner_line(state: &TuiState, width: usize) -> String {
    // An empty field draws no separator. The banner once read `ρ rho   ·  · model ·`,
    // because the renderer joined four fields and nothing filled three of them.
    // Every field is filtered. A security review found this row joining four of them raw while every
    // other row had a filter. The directory is the realistic vector: a name on a Unix filesystem may
    // hold an escape byte, so running rho inside a hostile checkout would put it on the banner. Git
    // rejects a control character in a ref name, and a model id comes from a flag or a config file.
    //
    // Nothing escaped today, because ratatui drops an escape from a cell. That is a second filter and
    // it is not rho's, so the invariant holds here instead of resting on a dependency.
    let parts = [
        sanitize_line(&state.cwd),
        sanitize_line(&state.branch),
        sanitize_line(&state.model),
        sanitize_line(&state.provider),
    ];
    let joined = parts
        .iter()
        .filter(|part| !part.trim().is_empty())
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(&format!(" {GLYPH_SEPARATOR} "));
    let text = if joined.is_empty() {
        BRAND.to_string()
    } else {
        format!("{BRAND}  {joined}")
    };
    pad(&text, width)
}

// ---- The transcript rows. -------------------------------------------------

/// Render one transcript row into its lines.
fn push_row(
    out: &mut Vec<StyledLine>,
    state: &TuiState,
    index: usize,
    row: &Row,
    width: usize,
    measure: usize,
) {
    match row {
        Row::User { text } => {
            // A submitted prompt sits on a band, so the eye finds where each turn began. Both
            // Claude Code and pi mark it the same way. The row is padded to the full width, so the
            // band reaches the frame edge instead of stopping at the last word. See
            // `D-a-submitted-prompt-sits-on-a-band`.
            let band = style_for(Role::UserBand);
            let wrapped = wrap_block(&sanitize_block(text), measure.saturating_sub(2));
            for (line_index, line) in wrapped.iter().enumerate() {
                let body = if line_index == 0 {
                    format!("{GLYPH_USER} {line}")
                } else {
                    format!("  {line}")
                };
                // Deliberately not padded here. `put` fills the tail of a row with the row's own
                // style, so the band reaches the frame edge through one mechanism instead of two.
                // Padding here as well would leave that fill untested, and untested code is where
                // this project's defects have lived.
                out.push(one((body, band)));
            }
        }
        Row::Assistant { text } => {
            // Markdown becomes colour, not punctuation. The line kind is stripped before the
            // wrap, and inline emphasis is scanned into runs so a wrap keeps every style. See
            // `D-markdown-line-level-first` and `SPEC-tui-markdown`.
            for line in scan_markdown(&sanitize_block(text)) {
                let base = style_for(markdown_role(line.kind));
                if line.kind == MarkdownKind::Rule {
                    out.push(one((rule_row(width), base)));
                    continue;
                }
                // A table row is already aligned, so it is drawn as it stands. Wrapping it
                // would stack the columns into nonsense, and `put` cuts it at the screen edge.
                if matches!(
                    line.kind,
                    MarkdownKind::TableHead | MarkdownKind::TableRule | MarkdownKind::TableRow
                ) {
                    out.push(one((pad(&line.text, width), base)));
                    continue;
                }
                // A code line is verbatim: never inline-scanned, never re-wrapped.
                if line.kind == MarkdownKind::CodeBlock {
                    out.push(one((pad(&line.text, width), base)));
                    continue;
                }
                // The fast path. A line with no inline marker has exactly one run, so it takes
                // the cheap `&str` wrap and one styled row. Prose is the common case, and the
                // per-character run wrap costs about eight times the allocations for no gain on
                // it. Measured: the frame benchmark went from 300 allocations to 2562 without
                // this, on a transcript with no markup at all. See `SPEC-tui-markdown` section 5.
                if !has_inline_markup(&line.text) {
                    let rows = wrap_block(&line.text, measure);
                    if rows.is_empty() {
                        out.push(one((blank(width), base)));
                    }
                    for row in rows {
                        out.push(one((row, base)));
                    }
                    continue;
                }
                let plain_base = markdown_role(line.kind) == Role::Text;
                let runs = inline_runs(&line.text, base, plain_base);
                let wrapped = wrap_runs(&runs, measure);
                if wrapped.is_empty() {
                    out.push(one((blank(width), base)));
                }
                for row in wrapped {
                    out.push(row);
                }
            }
        }
        Row::Thinking { text } => {
            // Reasoning draws in `Role::Muted`, never `Role::Text`, because it is the model's
            // private work and must not read as the answer. The mode decides how much shows.
            // See `SPEC-reasoning-across-providers` section 5.
            let summary = match format_duration(row_duration(state, index)) {
                Some(span) => format!("{GLYPH_THINKING} thought for {span}"),
                None => format!("{GLYPH_THINKING} thinking"),
            };
            let draw_text = |out: &mut Vec<StyledLine>| {
                // The whole text is wrapped, not a tail of it.
                //
                // This branch sliced the tail, because its renderer drew a fixed band of
                // fourteen rows and could never show an older line. That saved real work: a
                // frame went from 2769 microseconds to 95 at 504 kB. **The premise is gone.**
                // This renderer scrolls the transcript and can repaint any row, so an older
                // line is reachable and a tail would lose it. Correctness first, and the cost
                // needs measuring again on this renderer. See `docs/benchmarks.md`.
                for line in wrap(&sanitize_line(text), measure) {
                    out.push(one((pad(&line, width), style_for(Role::Muted))));
                }
            };
            match state.reasoning_display {
                ReasoningDisplay::Off => {}
                ReasoningDisplay::Summary => {
                    out.push(one((pad(&summary, width), style_for(Role::Muted))));
                }
                ReasoningDisplay::Full => {
                    out.push(one((pad(&summary, width), style_for(Role::Muted))));
                    draw_text(out);
                }
                ReasoningDisplay::Live => {
                    // A settled span means the answer started, so the reasoning collapses to
                    // the summary row. While it streams, the text shows.
                    if row_duration(state, index).is_some() {
                        out.push(one((pad(&summary, width), style_for(Role::Muted))));
                    } else {
                        draw_text(out);
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
            out.push(one((
                tool_header(state, index, name, preview, *status, width),
                text_style(),
            )));
            if row_fold(state, index) == RowFold::Expanded {
                for line in tool_body(state, index, preview) {
                    out.push(one((
                        pad(&format!("    {}", sanitize_line(&line)), width),
                        style_for(Role::Muted),
                    )));
                }
            }
        }
        Row::Error { message, detail } => {
            out.push(one((
                pad(
                    &format!(
                        "{GLYPH_ERROR} error {GLYPH_SEPARATOR} {}",
                        sanitize_line(message)
                    ),
                    width,
                ),
                style_for(Role::Error),
            )));
            for line in detail {
                out.push(one((
                    pad(&format!("     {}", sanitize_line(line)), width),
                    style_for(Role::Muted),
                )));
            }
        }
        Row::Notice { message } => {
            // A notice wraps. The real skill notice ends with its action, "Pass --trust-project to
            // load them", and a padded single line clipped exactly that.
            let head = format!("{GLYPH_NOTICE} notice {GLYPH_SEPARATOR} ");
            let style = style_for(Role::Warn);
            let text = sanitize_block(message);
            let indented = measure.saturating_sub(head.width());
            let longest = text
                .split_whitespace()
                .map(UnicodeWidthStr::width)
                .max()
                .unwrap_or(0);
            // The label keeps its own column only when the text still reads well beside it: wide
            // enough, and wide enough for the longest word. `wrap` breaks a word that cannot fit,
            // and breaking `--trust-project` at column 12 when the frame has 24 would split a word
            // that had room. So a notice whose longest word does not fit the indented column gives
            // the label its own row and takes the whole measure.
            if indented >= NOTICE_MIN_TEXT && longest <= indented {
                for (line_index, line) in wrap(&text, indented).iter().enumerate() {
                    let row = if line_index == 0 {
                        format!("{head}{line}")
                    } else {
                        format!("{}{line}", " ".repeat(head.width()))
                    };
                    out.push(one((pad(&row, width), style)));
                }
            } else {
                // Too narrow to keep a label column. The label takes its own row, and the text
                // takes the whole measure, because the text is the part that matters.
                out.push(one((pad(head.trim_end(), width), style)));
                for line in wrap(&text, measure.max(1)) {
                    out.push(one((pad(&line, width), style)));
                }
            }
        }
        Row::Agent { name, outcome, .. } => {
            out.push(one((
                pad(
                    &format!("agent {} {}", sanitize_line(name), sanitize_line(outcome)),
                    width,
                ),
                text_style(),
            )));
        }
        Row::Task {
            command,
            state: task,
            ..
        } => {
            out.push(one((
                pad(
                    &format!("task {} {}", sanitize_line(command), sanitize_line(task)),
                    width,
                ),
                text_style(),
            )));
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
    // The name is filtered here, at the boundary, and not only where a row is stored. A
    // second-opinion review found a **second** construction site for `Row::Tool` that stored a raw
    // name, and `Row` is a public enum, so another frontend can build one directly. Filtering the
    // payload and trusting the name was the shape of the mistake.
    let left = format!(
        "  {} {}  {}",
        status_glyph(status),
        sanitize_line(name),
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
/// True when the transcript holds no conversation. A notice is chrome, not conversation,
/// so it must not empty the splash. Every startup in a repository with skills raises a
/// notice, so gating the splash on `rows.is_empty()` would have retired it for good. See
/// `D-a-notice-reaches-the-transcript`.
fn conversation_is_empty(state: &TuiState) -> bool {
    state
        .rows
        .iter()
        .all(|row| matches!(row, Row::Notice { .. }))
}

/// The notice rows as display lines, in order, styled the same way the transcript styles
/// them. One source for the style, so the splash and the transcript cannot drift.
fn notice_lines(state: &TuiState, width: usize) -> Vec<StyledLine> {
    let measure = width.saturating_sub(RAIL_COLUMN).max(1);
    let mut out: Vec<StyledLine> = Vec::new();
    for (index, row) in state.rows.iter().enumerate() {
        if matches!(row, Row::Notice { .. }) {
            push_row(&mut out, state, index, row, width, measure);
        }
    }
    out
}

fn empty_state(state: &TuiState, width: usize, rows: usize) -> Option<Vec<StyledLine>> {
    let accent = style_for(Role::Accent);
    let muted = style_for(Role::Muted);
    let art_indent = width.saturating_sub(4) / 2;
    let starter_indent = width.saturating_sub(STARTER_BLOCK_WIDTH) / 2;

    let mut block: Vec<StyledLine> = Vec::new();
    for art in BRAND_ART {
        block.push(one((
            pad(&format!("{}{art}", " ".repeat(art_indent)), width),
            accent,
        )));
    }
    block.push(one((blank(width), text_style())));
    block.push(one((centre(WORDMARK, width), text_style())));
    block.push(one((blank(width), text_style())));
    let session = format!(
        "{} {GLYPH_SEPARATOR} {} {GLYPH_SEPARATOR} ready",
        state.model, state.provider
    );
    block.push(one((centre(&session, width), muted)));
    block.push(one((blank(width), text_style())));
    for (key, outcome) in STARTERS {
        let line = format!("{}{:<8}{outcome}", " ".repeat(starter_indent), key);
        block.push(one((pad(&line, width), text_style())));
    }

    // A notice draws under the starters, because the user reads the screen top down and a
    // notice is the thing rho wants read. When the notices cannot fit, the caller falls
    // back to the scrollable transcript, so a notice is never truncated away.
    let notices = notice_lines(state, width);
    if !notices.is_empty() {
        block.push(one((blank(width), text_style())));
        block.extend(notices);
    }
    if block.len() > rows {
        return None;
    }

    // Centre the block vertically, biased one row up when the gap is odd, matching
    // the design frame.
    let top = rows.saturating_sub(block.len()).div_ceil(2);
    let mut out: Vec<StyledLine> = Vec::with_capacity(rows);
    for _ in 0..top {
        out.push(one((blank(width), text_style())));
    }
    out.append(&mut block);
    while out.len() < rows {
        out.push(one((blank(width), text_style())));
    }
    out.truncate(rows);
    Some(out)
}

/// Centre `text` in `width` columns, floor-biased to the left, matching the design.
fn centre(text: &str, width: usize) -> String {
    let indent = width.saturating_sub(text.width()) / 2;
    pad(&format!("{}{text}", " ".repeat(indent)), width)
}

// ---- The panels. ----------------------------------------------------------

/// The rows a panel wants, and the rows it never yields. The transcript takes what is
/// left, so a panel takes its whole content on a full screen. An approval yields nothing,
/// because every row of it is safety information, including the session root. See
/// `D-ledger-wins-the-band` and `SPEC-tui-alternate-screen` section 6.
fn panel_demand(state: &TuiState) -> (usize, usize) {
    match &state.panel {
        Panel::None => (0, 0),
        // An approval states every row and yields none.
        Panel::Approval(_) => (APPROVAL_ROWS, APPROVAL_ROWS),
        // The help draws the whole binding table, because the screen has room. It yields
        // to the transcript on a short screen, because a truncated help is not unsafe.
        Panel::Help => (bindings().len(), 0),
        Panel::SlashList(list) => (filter_slash_commands(&list.query).len(), 0),
        Panel::HistorySearch(search) => {
            let matches = filter_history(&state.history, &search.query).len();
            // The matches, or one row saying there were none, plus the query row.
            (matches.clamp(1, HISTORY_MATCH_ROWS) + 1, 0)
        }
    }
}

/// The approval panel is four rows: the request, the command, the root, and the choices.
const APPROVAL_ROWS: usize = 4;

fn panel_lines(state: &TuiState, width: usize, budget: usize) -> Vec<StyledLine> {
    if budget == 0 {
        return Vec::new();
    }
    let mut lines = match &state.panel {
        Panel::None => Vec::new(),
        Panel::Approval(approval) => approval_panel(approval, width),
        Panel::SlashList(list) => slash_panel(list, width),
        Panel::Help => help_panel(width),
        Panel::HistorySearch(search) => history_search_panel(search, &state.history, width),
    };
    // A panel never overruns its grant. This is the backstop that keeps the arithmetic
    // honest when the screen is short.
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
) -> Vec<StyledLine> {
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
        lines.push(one((pad(&body, width), style)));
    }
    if matches.is_empty() {
        // A search that found nothing must say so, because an empty panel reads as a
        // defect. This is the house rule from `D-a-panel-nobody-can-open`.
        lines.push(one((pad("   no match in this session", width), muted)));
    }
    lines.push(one((
        pad(
            &format!(
                "  search {GLYPH_SEPARATOR} {}",
                sanitize_line(&search.query)
            ),
            width,
        ),
        text_style(),
    )));
    lines
}

/// The approval panel: the request, the command, the root, and the choices.
///
/// Four rows, and it yields none of them. A command means nothing without the tree it acts
/// on, so the root is not decoration. See `D-ledger-wins-the-band`.
fn approval_panel(approval: &Approval, width: usize) -> Vec<StyledLine> {
    let caution = style_for(Role::Caution);
    let muted = style_for(Role::Muted);
    let slot = duration_slot(approval.millis);
    let left = format!("  {GLYPH_APPROVAL} {}", sanitize_line(&approval.title));
    vec![
        one((justify(&left, &slot, width), caution)),
        one((
            pad(
                &format!("  {GLYPH_QUOTE} {}", sanitize_line(&approval.command)),
                width,
            ),
            text_style(),
        )),
        one((
            pad(
                &format!(
                    "  {GLYPH_QUOTE}  the session root is {}",
                    sanitize_line(&approval.root)
                ),
                width,
            ),
            muted,
        )),
        one((pad(&format!("  {APPROVAL_CHOICES}"), width), caution)),
    ]
}

fn slash_panel(list: &SlashList, width: usize) -> Vec<StyledLine> {
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
        lines.push(one((pad(&body, width), style)));
    }
    lines
}

/// The help panel: the whole binding table, one row per key.
///
/// rho owns the whole screen now, so the panel draws every binding and needs no window,
/// no counted header, and no scroll. The old window showed eight rows and drew zero when
/// the band could not grant its header row. See `SPEC-tui-alternate-screen` section 6.
///
/// The rows come from `bindings()`, never from a literal, so adding or removing a binding
/// changes the help with no other edit.
fn help_panel(width: usize) -> Vec<StyledLine> {
    let muted = style_for(Role::Muted);
    bindings()
        .iter()
        .map(|binding| {
            // An unwired binding says so, so the help never promises a key that answers
            // nothing. See `D-a-panel-nobody-can-open`.
            let (note, style) = if binding.built {
                ("", text_style())
            } else {
                (" · not built yet", muted)
            };
            one((
                pad(
                    &format!("  {:<15}{}{note}", binding.keys, binding.summary),
                    width,
                ),
                style,
            ))
        })
        .collect()
}

// ---- The composer. --------------------------------------------------------

/// The composer box: a rounded frame around the draft or the placeholder.
///
/// It draws `display_lines`, so a tall draft wraps and scrolls inside the box. It places
/// the cursor glyph at `cursor_cell`, so the cursor follows the edit and the wrap.
fn composer_lines(state: &TuiState, width: usize) -> Vec<StyledLine> {
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

    let mut lines = vec![one((rule_line(width), muted))];
    for (index, row) in rows.into_iter().enumerate() {
        // The placeholder is muted, as `docs/tui-design.md` section 8 states. It drew in
        // the default foreground, which reads as bright as the assistant's answer.
        let style = if show_placeholder && index == 0 {
            muted
        } else {
            text_style()
        };
        lines.push(one((pad(&row, width), style)));
    }
    lines.push(one((rule_line(width), muted)));
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
        Panel::Help => "esc close · / lists commands",
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
        // A turn cap counts provider round trips, so it cannot bound a turn that asks
        // for forty tools. This is the other cap, and the footer names it plainly.
        MaxToolCalls => "max tool calls",
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
fn put(frame: &mut Frame<'_>, y: usize, width: usize, line: &StyledLine) {
    // The row `y` counts from the top of the band, and the band is not the screen. An
    // inline viewport anchors to the cursor row, so `Frame::area()` carries the origin.
    // A write at an absolute (0, 0) panics there, and every fullscreen fixture missed it.
    //
    // **This is where the row's width invariant is enforced**, and it is the only place. A run that
    // would cross the right edge is clipped, and a row short of the width is padded to it. So no
    // producer can overflow the row, and none has to remember to pad. See
    // `crates/rho-tui/src/styled.rs`.
    //
    // Two reviews reached opposite conclusions about the clip, and the disagreement is recorded here
    // rather than settled by picking the cheaper one.
    //
    // A test-quality audit called it dead: `set_stringn` clamps to the buffer edge on its own, so no
    // mutation of `room` changes an observable cell, and it could not be pinned by a test. By its
    // rule, untested code should go.
    //
    // A security review, on the same day, found the opposite failure in `banner_line`: rho was
    // leaning on ratatui to drop an escape and calling that a defence. A guarantee that lives in a
    // dependency changes when the dependency changes, and it does not travel to a log, a clipboard
    // write, or another backend.
    //
    // The clip stays for the second reason. It costs one comparison per run, it makes `put`'s promise
    // true in rho's own code, and `a_table_wider_than_the_screen_is_cut_not_wrapped` exercises the
    // path even though it cannot distinguish rho's clip from ratatui's. That last clause is the
    // honest part: this is defence in depth, not a tested guarantee.
    let area = frame.area();
    let row = area.y + y as u16;
    let mut column = 0usize;
    for (text, style) in line {
        if column >= width {
            break;
        }
        let room = width - column;
        frame
            .buffer_mut()
            .set_stringn(area.x + column as u16, row, text, room, *style);
        column += text.width().min(room);
    }
    if column < width {
        // The tail carries the row's own style, not the default. A banded row whose text ends early
        // would otherwise draw a ragged stripe that stops at the last word.
        let fill = line.last().map(|(_, style)| *style).unwrap_or_default();
        frame.buffer_mut().set_stringn(
            area.x + column as u16,
            row,
            blank(width - column),
            width - column,
            fill,
        );
    }
}

/// Write one cell at column `col`, row `y` from the frame top, for the scroll rail.
fn put_char(frame: &mut Frame<'_>, y: usize, col: usize, text: &str, style: Style) {
    let area = frame.area();
    let x = area.x + col as u16;
    let row = area.y + y as u16;
    frame.buffer_mut().set_stringn(x, row, text, 1, style);
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

/// The styled runs of one line, with its inline markers removed.
///
/// `base` is the line's own style, from its markdown kind. Bold and italic add a modifier to
/// it, and a code span takes the code role instead, because a colour reads more clearly than a
/// third modifier. Measured against pi, which colours inline code and leaves emphasis as
/// modifiers.
fn inline_runs(text: &str, base: Style, plain_base: bool) -> StyledLine {
    let code = style_for(Role::MdCodeBlock);
    scan_inline(text)
        .into_iter()
        .filter(|run| !run.text.is_empty())
        .map(|run| {
            // Emphasis takes a colour as well as a modifier, because a terminal may draw no
            // italic at all and may draw bold at the same weight. Only inside body text: a
            // heading and a quote carry their own colour, and repainting a word inside one
            // would look like a defect. So there the modifier carries it alone.
            let mut style = if run.code {
                code
            } else if plain_base && run.bold {
                style_for(Role::MdBold)
            } else if plain_base && run.italic {
                style_for(Role::MdItalic)
            } else {
                base
            };
            if run.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if run.italic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            (run.text, style)
        })
        .collect()
}

/// Wrap styled runs to `width`, keeping each run's style across the break.
///
/// Wrapping has to happen **over runs**, not over text. The contract review found the ordering
/// bug: wrapping raw text and then removing `**` leaves the row narrower than the wrap assumed.
/// Working on runs whose markers are already gone keeps the measure honest.
///
/// The pass is per character, which is cheap at a text measure of 80 columns, and it coalesces
/// neighbouring characters of one style back into one run.
fn wrap_runs(runs: &StyledLine, width: usize) -> Vec<StyledLine> {
    if width == 0 {
        return Vec::new();
    }
    // Flatten to characters, each carrying its style. The leading indent is kept, so a wrapped
    // code-ish line stays legible.
    let mut cells: Vec<(char, Style)> = Vec::new();
    for (text, style) in runs {
        for ch in text.chars() {
            cells.push((ch, *style));
        }
    }
    if cells.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<StyledLine> = Vec::new();
    let mut row: Vec<(char, Style)> = Vec::new();
    let mut word: Vec<(char, Style)> = Vec::new();
    let mut row_width = 0usize;
    let mut word_width = 0usize;

    // Push the finished row, coalescing runs of one style.
    let flush_row = |row: &mut Vec<(char, Style)>, rows: &mut Vec<StyledLine>| {
        if row.is_empty() {
            return;
        }
        let mut line: StyledLine = Vec::new();
        for (ch, style) in row.drain(..) {
            match line.last_mut() {
                Some((text, last)) if *last == style => text.push(ch),
                _ => line.push((ch.to_string(), style)),
            }
        }
        rows.push(line);
    };

    for (ch, style) in cells {
        let cw = ch.to_string().width();
        if ch == ' ' {
            // A space closes the pending word onto the row.
            if row_width + word_width > width && row_width > 0 {
                flush_row(&mut row, &mut rows);
                row_width = 0;
            }
            row.append(&mut word);
            row_width += word_width;
            word_width = 0;
            if row_width + cw <= width {
                row.push((ch, style));
                row_width += cw;
            }
            continue;
        }
        // A word longer than the row must break. The break happens **before** the character that
        // would tip it over, so no row is ever wider than the frame. Appending first made a row up
        // to two columns too wide, and a wide glyph at that edge was then dropped by `put`.
        if word_width + cw > width {
            if row_width > 0 {
                flush_row(&mut row, &mut rows);
                row_width = 0;
            }
            if !word.is_empty() {
                row.append(&mut word);
                flush_row(&mut row, &mut rows);
                word_width = 0;
            }
        }
        word.push((ch, style));
        word_width += cw;
    }
    if row_width + word_width > width && row_width > 0 {
        flush_row(&mut row, &mut rows);
    }
    row.append(&mut word);
    flush_row(&mut row, &mut rows);
    // Trailing spaces are padding, and `put` owns padding.
    for line in &mut rows {
        while line
            .last()
            .is_some_and(|(text, _)| text.chars().all(|ch| ch == ' '))
        {
            line.pop();
        }
        if let Some((text, _)) = line.last_mut() {
            while text.ends_with(' ') {
                text.pop();
            }
        }
    }
    rows
}

/// The colour role for one markdown kind. One arm per kind, so a new kind cannot be added
/// without answering for its colour.
fn markdown_role(kind: MarkdownKind) -> Role {
    match kind {
        MarkdownKind::Text => Role::Text,
        MarkdownKind::Heading => Role::MdHeading,
        MarkdownKind::Fence => Role::Muted,
        MarkdownKind::CodeBlock => Role::MdCodeBlock,
        MarkdownKind::Quote => Role::MdQuote,
        // The item text keeps the body colour. Measured against pi, which colours only the
        // marker and leaves the text default: colouring a whole item accent was louder than
        // the prior art and harder to read, and phase 1 cannot colour a glyph alone.
        MarkdownKind::Bullet => Role::Text,
        MarkdownKind::Rule => Role::Muted,
        // A table's header is bold and bright, its rule is quiet, and its data reads as body
        // text. Measured against jcode, which does the same.
        MarkdownKind::TableHead => Role::MdBold,
        MarkdownKind::TableRule => Role::Muted,
        MarkdownKind::TableRow => Role::Text,
    }
}

/// A horizontal rule, drawn across the text measure.
fn rule_row(width: usize) -> String {
    // A divider spans the frame. It was capped at the old reading measure, which left it short.
    pad(&GLYPH_RULE.repeat(width), width)
}

/// Wrap a block of text, one source line at a time, so a line break survives.
///
/// `wrap` alone re-flows across every newline, because it splits on whitespace. That turned
/// a markdown list and a fenced code block into one paragraph. This keeps each source line,
/// keeps a blank line between paragraphs, and still wraps a line too long for the width.
///
/// **The indent is kept.** `wrap` drops leading spaces, so a nested code line came out flat.
/// A continuation row is indented to match its source line, so wrapped code stays legible.
/// A trailing blank line is dropped, because a model answer usually ends with a newline and
/// a gap above the composer looks like a defect. See `D-block-text-keeps-its-shape`.
fn wrap_block(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for source in text.split('\n') {
        let body = source.trim_start_matches(' ');
        let indent_cols = source.len() - body.len();
        if body.is_empty() {
            out.push(String::new());
            continue;
        }
        if indent_cols + body.width() <= width {
            out.push(source.to_string());
            continue;
        }
        let room = width.saturating_sub(indent_cols).max(1);
        let indent = " ".repeat(indent_cols);
        for piece in wrap(body, room) {
            out.push(format!("{indent}{piece}"));
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Greedy word wrap to `width` display columns. A single space joins words.
///
/// **A word wider than the row is broken**, by display column, instead of being emitted whole. It
/// used to be emitted whole and the renderer then clipped it to the frame, dropping the tail with
/// no marker. A URL, a path, a hash, and a stack-trace line are all one long word, and so is a
/// whole CJK or Thai paragraph, because those scripts put no spaces between words. Measured before
/// the fix: 30 characters at width 10 drew 10, and a 60 glyph CJK paragraph at width 80 drew 40.
/// See `D-a-long-word-breaks`.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if word.width() > width {
            // The word cannot fit any row, so break it. Whatever is on the current row goes first.
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            let mut chunk_width = 0usize;
            for ch in word.chars() {
                let cell = ch.width().unwrap_or(0);
                if chunk_width + cell > width && !chunk.is_empty() {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_width = 0;
                }
                chunk.push(ch);
                chunk_width += cell;
            }
            current = chunk;
            continue;
        }
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
    let mut style = match role_256(role) {
        Some(index) => Style::default().fg(Color::Indexed(index)),
        None => Style::default(),
    };
    if let Some(index) = role_bg_256(role) {
        style = style.bg(Color::Indexed(index));
    }
    // The bold weight comes from the role table, not from a name in this function. It used to
    // read `if role == Role::Caution`, so every new bold role needed an edit to shared code.
    // `role_16` already states the weight for every role, and the exhaustive match there means
    // a new role cannot forget it. This changes no existing appearance: `Caution` is the only
    // old role the table marks bold.
    let table = role_16(role);
    if table.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if table.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    style
}
