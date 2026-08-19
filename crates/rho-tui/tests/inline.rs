//! Inline emphasis becomes style, not punctuation.
//!
//! Phase 1 coloured whole rows. This is the part a reader notices, because `**bold**` and
//! `` `code` `` still showed their markers on screen. Measured against pi and jcode, both style
//! these, and rho did not. See `docs/verification/notices-live.md`.
//!
//! **Every rule here exists to avoid a false positive on code-heavy prose.** The contract
//! review found that a naive pair rule italicises the `3` in `2 * 3 * 4`, so the flanking rule
//! is not optional. See `SPEC-tui-markdown` section 3a item 3.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use rho_tui::{Row, TuiState, render, scan_inline};

/// The runs as `(text, bold, italic, code)`, for a compact assertion.
fn runs(text: &str) -> Vec<(String, bool, bool, bool)> {
    scan_inline(text)
        .into_iter()
        .map(|run| (run.text, run.bold, run.italic, run.code))
        .collect()
}

/// The text of every run joined, which must equal the visible text.
fn visible(text: &str) -> String {
    scan_inline(text).into_iter().map(|r| r.text).collect()
}

// ---- The markers come off, and the style goes on. -------------------------------

#[test]
fn a_line_with_no_markup_is_one_plain_run() {
    assert_eq!(
        runs("just a sentence."),
        vec![("just a sentence.".to_string(), false, false, false)]
    );
}

#[test]
fn bold_markers_are_removed_and_the_run_is_bold() {
    assert_eq!(
        runs("a **strong** word"),
        vec![
            ("a ".to_string(), false, false, false),
            ("strong".to_string(), true, false, false),
            (" word".to_string(), false, false, false),
        ]
    );
    // An underscore is NOT emphasis in rho. This is a deliberate deviation from CommonMark,
    // which would render `__strong__` as bold. In a coding agent's prose an underscore is an
    // identifier character, and eating `__init__` or `__all__` would be worse than leaving a
    // pair of underscores on screen. See `D-markdown-line-level-first`.
    assert_eq!(visible("a __strong__ word"), "a __strong__ word");
}

#[test]
fn italic_markers_are_removed_and_the_run_is_italic() {
    assert_eq!(
        runs("a *slanted* word"),
        vec![
            ("a ".to_string(), false, false, false),
            ("slanted".to_string(), false, true, false),
            (" word".to_string(), false, false, false),
        ]
    );
}

#[test]
fn inline_code_drops_its_backticks_and_takes_the_code_flag() {
    assert_eq!(
        runs("call `wrap_block()` now"),
        vec![
            ("call ".to_string(), false, false, false),
            ("wrap_block()".to_string(), false, false, true),
            (" now".to_string(), false, false, false),
        ]
    );
}

#[test]
fn a_triple_marker_is_bold_and_italic() {
    assert_eq!(
        runs("***both***"),
        vec![("both".to_string(), true, true, false)]
    );
}

// ---- The false positives. This is the important half. ---------------------------

#[test]
fn arithmetic_is_not_italic() {
    // The review's example. A naive pair rule italicises the ` 3 `. The flanking rule says an
    // opening marker is not followed by a space and a closing marker is not preceded by one.
    assert_eq!(visible("2 * 3 * 4"), "2 * 3 * 4");
    assert_eq!(runs("2 * 3 * 4").len(), 1, "one plain run, no emphasis");
    assert_eq!(visible("2 ** 3 ** 4"), "2 ** 3 ** 4");
}

#[test]
fn an_underscore_inside_a_word_is_not_italic() {
    // `wrap_block`, `snake_case`, and `__init__` are identifiers, not emphasis.
    for text in [
        "wrap_block and scan_inline",
        "a snake_case_name here",
        "call __init__ first",
    ] {
        assert_eq!(visible(text), text, "identifiers survive: {text}");
    }
}

#[test]
fn an_unclosed_marker_stays_text() {
    for text in [
        "a **bold that never closes",
        "a *slant that never closes",
        "a `code that never closes",
        "5 * 3 = 15",
    ] {
        assert_eq!(visible(text), text, "unclosed markup is text: {text}");
    }
}

#[test]
fn a_backslash_escapes_a_marker() {
    // `\*` is a literal star, and the backslash itself is consumed.
    assert_eq!(visible(r"a \*literal\* star"), "a *literal* star");
    assert_eq!(runs(r"a \*literal\* star").len(), 1, "no emphasis at all");
}

#[test]
fn nothing_is_markup_inside_inline_code() {
    // A code span is verbatim. `**` inside it is two stars.
    assert_eq!(
        runs("`a ** b`"),
        vec![("a ** b".to_string(), false, false, true)]
    );
    assert_eq!(visible("`--no-mouse`"), "--no-mouse");
}

#[test]
fn a_double_backtick_span_may_hold_a_backtick() {
    // CommonMark: a longer fence lets the span contain the shorter one.
    assert_eq!(
        runs("``a ` b``"),
        vec![("a ` b".to_string(), false, false, true)]
    );
}

#[test]
fn a_path_and_a_flag_survive_untouched() {
    // The text a coding agent writes constantly.
    for text in [
        "run --no-mouse to opt out",
        "see crates/rho-tui/src/render.rs",
        "the C_FLAG and the D_FLAG",
        "2*3*4 with no spaces",
    ] {
        assert_eq!(visible(text), text, "must survive: {text}");
    }
}

// ---- The invariant the contract needs. ------------------------------------------

#[test]
fn the_runs_are_the_visible_text_and_nothing_else() {
    // The property, not an example. For every input, the runs joined equal the input with its
    // markers removed, and no character is invented. This is what lets `put` trust the width.
    let corpus = [
        "plain",
        "a **b** c",
        "a *b* c",
        "a `b` c",
        "***x***",
        "2 * 3 * 4",
        "wrap_block",
        r"a \*b\* c",
        "``a ` b``",
        "**unclosed",
        "`code with **stars**`",
        "",
        "**a** and *b* and `c`",
    ];
    for text in corpus {
        let joined = visible(text);
        let allowed: std::collections::HashSet<char> = text.chars().collect();
        for ch in joined.chars() {
            assert!(
                allowed.contains(&ch),
                "the scanner invented {ch:?} from {text:?}"
            );
        }
        assert!(
            joined.chars().count() <= text.chars().count(),
            "markers are removed, never added: {text:?} -> {joined:?}"
        );
    }
}

// ---- What the user sees. --------------------------------------------------------

fn drawn(text: &str, width: u16) -> Vec<(String, Vec<ratatui::style::Style>)> {
    let mut state = TuiState::default();
    state.rows.push(Row::Assistant {
        text: text.to_string(),
    });
    let rows = 24u16;
    let backend = TestBackend::new(width, rows);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..rows)
        .map(|y| {
            let line: String = (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>();
            let styles = (0..width).map(|x| buffer[(x, y)].style()).collect();
            (line, styles)
        })
        .collect()
}

/// The style of the first cell of `needle` in the drawn rows.
fn style_at(rows: &[(String, Vec<ratatui::style::Style>)], needle: &str) -> ratatui::style::Style {
    for (line, styles) in rows {
        if let Some(index) = line.find(needle) {
            return styles[line[..index].chars().count()];
        }
    }
    panic!("{needle:?} was not drawn");
}

#[test]
fn a_bold_word_draws_bold_and_its_stars_are_gone() {
    let rows = drawn("this is **strong** here", 60);
    let joined: String = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert!(!joined.contains("**"), "no stars on screen: {joined:?}");
    assert!(
        style_at(&rows, "strong")
            .add_modifier
            .contains(Modifier::BOLD),
        "the word is bold"
    );
    assert!(
        !style_at(&rows, "this")
            .add_modifier
            .contains(Modifier::BOLD),
        "the prose around it is not"
    );
}

#[test]
fn inline_code_draws_in_its_own_colour_without_backticks() {
    let rows = drawn("call `render()` now", 60);
    let joined: String = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert!(!joined.contains('`'), "no backticks on screen: {joined:?}");
    assert_ne!(
        style_at(&rows, "render()").fg,
        style_at(&rows, "call").fg,
        "code has its own colour"
    );
}

#[test]
fn emphasis_survives_a_wrap() {
    // Wrapping happens over runs now, so a bold word that lands past the wrap point keeps its
    // style. Under the old order the markers were counted in the width and the style was lost.
    let text = format!("{} **strong** tail", "filler ".repeat(12));
    let rows = drawn(&text, 40);
    let joined: String = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert!(!joined.contains("**"), "no stars survive a wrap");
    assert!(
        style_at(&rows, "strong")
            .add_modifier
            .contains(Modifier::BOLD),
        "bold survives the wrap"
    );
    for (line, _) in rows.iter().filter(|(l, _)| !l.trim().is_empty()) {
        assert!(
            line.chars().count() <= 40,
            "no row exceeds the width: {line:?}"
        );
    }
}

#[test]
fn a_bullet_keeps_its_glyph_and_styles_its_inline_markup() {
    // The combination phase 1 could not do: a line-level kind plus inline runs inside it.
    let rows = drawn("- item with **bold** and `code`", 60);
    let joined: String = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert!(joined.contains('\u{2022}'), "the bullet glyph draws");
    assert!(!joined.contains("**"), "no stars");
    assert!(!joined.contains('`'), "no backticks");
    assert!(
        style_at(&rows, "bold")
            .add_modifier
            .contains(Modifier::BOLD),
        "bold inside a bullet is bold"
    );
}

#[test]
fn a_code_block_line_is_never_inline_scanned() {
    // Inside a fence, `**` is two stars and stays on screen.
    let rows = drawn("```\nlet x = a ** b;\n```", 60);
    let joined: String = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert!(
        joined.contains("a ** b"),
        "code keeps its stars: {joined:?}"
    );
}

// ---- Emphasis must be visible by colour, not only by a modifier. ----------------

#[test]
fn bold_and_italic_carry_a_colour_as_well_as_a_modifier() {
    // A modifier alone is not enough. Plenty of terminals draw no italic at all, and some draw
    // bold as the same weight, so emphasis would vanish. Each also takes a colour, so the
    // meaning survives a terminal that ignores the modifier.
    let rows = drawn("plain **strong** and *slanted* here", 60);
    let plain = style_at(&rows, "plain");
    let bold = style_at(&rows, "strong");
    let italic = style_at(&rows, "slanted");

    assert!(bold.add_modifier.contains(Modifier::BOLD), "bold modifier");
    assert!(
        italic.add_modifier.contains(Modifier::ITALIC),
        "italic modifier"
    );
    assert_ne!(bold.fg, plain.fg, "bold is visible without its modifier");
    assert_ne!(
        italic.fg, plain.fg,
        "italic is visible without its modifier"
    );
    assert_ne!(bold.fg, italic.fg, "bold and italic are told apart");
}

#[test]
fn emphasis_inside_a_heading_keeps_the_heading_colour() {
    // A heading already carries a colour. A bold word inside it must not repaint itself with
    // the prose bold colour, or the heading would look broken in the middle.
    let rows = drawn("## A **strong** heading\nplain line", 60);
    let heading_word = style_at(&rows, "A ");
    let bold_word = style_at(&rows, "strong");
    assert_eq!(
        bold_word.fg, heading_word.fg,
        "the heading's colour wins inside a heading"
    );
    assert!(
        bold_word.add_modifier.contains(Modifier::BOLD),
        "and the bold modifier still applies"
    );
}
