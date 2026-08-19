//! Markdown becomes colour, not punctuation.
//!
//! rho drew a model answer as plain text, so `# heading`, a fence, and `> quote` arrived as
//! their own punctuation. The owner reads code all day and asked for colour instead.
//!
//! **This covers the line-level subset only.** A heading, a fence, a code line, a quote, a
//! bullet, and a rule each style a whole row, so the style is uniform and the markup can be
//! stripped *before* wrapping. Inline emphasis (`**bold**`, `` `code` ``) needs a run-level
//! contract and correct wrap-over-runs, which `SPEC-tui-markdown` phase 2 carries.
//!
//! The ordering matters and it is the reason for the split. `wrap_block` measures the text it
//! is given. Stripping `**` after wrapping would leave every affected row four columns
//! narrower than the wrap assumed. Stripping a line prefix before wrapping keeps the measure
//! honest. See `D-markdown-line-level-first`.
//!
//! See `SPEC-tui-markdown` sections 3 and 6.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use rho_tui::{MarkdownKind, Row, TuiState, render, scan_markdown};

// ---- The scanner, as a pure function. ------------------------------------------

fn kinds(text: &str) -> Vec<MarkdownKind> {
    scan_markdown(text)
        .into_iter()
        .map(|line| line.kind)
        .collect()
}

fn texts(text: &str) -> Vec<String> {
    scan_markdown(text)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

#[test]
fn a_row_with_no_markup_stays_text() {
    assert_eq!(kinds("just a sentence."), vec![MarkdownKind::Text]);
    assert_eq!(texts("just a sentence."), vec!["just a sentence."]);
}

#[test]
fn a_heading_drops_its_hashes_and_takes_the_heading_kind() {
    assert_eq!(kinds("## The plan"), vec![MarkdownKind::Heading]);
    assert_eq!(texts("## The plan"), vec!["The plan"]);
    // Every level, one to six.
    for level in 1..=6 {
        let line = format!("{} Title", "#".repeat(level));
        assert_eq!(kinds(&line), vec![MarkdownKind::Heading], "level {level}");
        assert_eq!(texts(&line), vec!["Title"], "level {level}");
    }
}

#[test]
fn a_hash_with_no_space_is_not_a_heading() {
    // `#[cfg(test)]` and `#42` are not headings. A heading needs a space after its hashes.
    assert_eq!(kinds("#[derive(Debug)]"), vec![MarkdownKind::Text]);
    assert_eq!(texts("#[derive(Debug)]"), vec!["#[derive(Debug)]"]);
    assert_eq!(kinds("#42 is the answer"), vec![MarkdownKind::Text]);
    assert_eq!(kinds("####### seven hashes"), vec![MarkdownKind::Text]);
}

#[test]
fn a_fence_line_and_its_body_take_their_own_kinds() {
    let block = "```rust\nfn main() {}\n```";
    assert_eq!(
        kinds(block),
        vec![
            MarkdownKind::Fence,
            MarkdownKind::CodeBlock,
            MarkdownKind::Fence
        ]
    );
    // The fence keeps its markers and its language, as pi draws it.
    assert_eq!(texts(block)[0], "```rust");
    assert_eq!(texts(block)[1], "fn main() {}");
}

#[test]
fn markup_inside_a_fence_is_not_markup() {
    // A star is multiplication, a hash is an attribute, and a dash is a minus. Code is code.
    let block = "```\n# not a heading\n- not a bullet\n> not a quote\n---\n```";
    assert_eq!(
        kinds(block),
        vec![
            MarkdownKind::Fence,
            MarkdownKind::CodeBlock,
            MarkdownKind::CodeBlock,
            MarkdownKind::CodeBlock,
            MarkdownKind::CodeBlock,
            MarkdownKind::Fence
        ]
    );
    // And nothing is stripped from a code line.
    assert_eq!(texts(block)[1], "# not a heading");
    assert_eq!(texts(block)[2], "- not a bullet");
}

#[test]
fn an_unclosed_fence_keeps_the_rest_as_code() {
    // A streamed answer is cut mid-block all the time. The tail must stay code, not turn
    // back into prose halfway through.
    let block = "```rust\nfn main() {\n    let x = 1;";
    assert_eq!(
        kinds(block),
        vec![
            MarkdownKind::Fence,
            MarkdownKind::CodeBlock,
            MarkdownKind::CodeBlock
        ]
    );
}

#[test]
fn a_bullet_becomes_a_glyph_and_keeps_its_text() {
    for marker in ["-", "*", "+"] {
        let line = format!("{marker} first item");
        assert_eq!(kinds(&line), vec![MarkdownKind::Bullet], "marker {marker}");
        assert_eq!(texts(&line), vec!["• first item"], "marker {marker}");
    }
    // An indented bullet keeps its indent, because nesting carries meaning.
    assert_eq!(texts("  - nested"), vec!["  • nested"]);
}

#[test]
fn a_dash_without_a_space_is_not_a_bullet() {
    // `--no-mouse` and `-42` are not lists.
    assert_eq!(
        kinds("--no-mouse turns capture off"),
        vec![MarkdownKind::Text]
    );
    assert_eq!(kinds("-42"), vec![MarkdownKind::Text]);
}

#[test]
fn a_numbered_item_keeps_its_number() {
    assert_eq!(kinds("1. first"), vec![MarkdownKind::Bullet]);
    assert_eq!(texts("1. first"), vec!["1. first"]);
    assert_eq!(kinds("12. twelfth"), vec![MarkdownKind::Bullet]);
    // A version string is not a list.
    assert_eq!(kinds("1.2.3 is the version"), vec![MarkdownKind::Text]);
}

#[test]
fn a_quote_takes_a_bar_and_the_quote_kind() {
    assert_eq!(kinds("> quoted text"), vec![MarkdownKind::Quote]);
    assert_eq!(texts("> quoted text"), vec!["┃ quoted text"]);
    // A shell redirect is not a quote, because it has no space.
    assert_eq!(kinds(">out.txt"), vec![MarkdownKind::Text]);
}

#[test]
fn a_rule_becomes_a_rule() {
    for marker in ["---", "***", "___", "-----"] {
        assert_eq!(kinds(marker), vec![MarkdownKind::Rule], "marker {marker}");
    }
    // Not a rule: fewer than three, or text after it.
    assert_eq!(kinds("--"), vec![MarkdownKind::Text]);
    assert_eq!(kinds("--- and then text"), vec![MarkdownKind::Text]);
}

#[test]
fn a_blank_line_stays_blank() {
    assert_eq!(
        kinds("one\n\ntwo"),
        vec![MarkdownKind::Text, MarkdownKind::Text, MarkdownKind::Text]
    );
    assert_eq!(texts("one\n\ntwo")[1], "");
}

// ---- What the user sees. --------------------------------------------------------

/// The drawn rows and the foreground of each row's first cell.
fn drawn(text: &str, width: u16) -> Vec<(String, ratatui::style::Style)> {
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
                .collect::<String>()
                .trim_end()
                .to_string();
            // The style of the first non-space cell, which carries the row's role.
            let x = (0..width)
                .find(|x| buffer[(*x, y)].symbol() != " ")
                .unwrap_or(0);
            (line, buffer[(x, y)].style())
        })
        .collect()
}

#[test]
fn a_heading_draws_without_hashes_and_stands_out() {
    let rows = drawn("# The plan\nplain text here", 60);
    let heading = rows
        .iter()
        .find(|(text, _)| text.trim() == "The plan")
        .expect("the heading draws with no hashes");
    let plain = rows
        .iter()
        .find(|(text, _)| text.trim() == "plain text here")
        .expect("the plain row draws");
    assert!(
        heading.1.add_modifier.contains(Modifier::BOLD),
        "a heading is bold: {:?}",
        heading.1
    );
    assert_ne!(
        heading.1.fg, plain.1.fg,
        "a heading must not look like body text"
    );
}

#[test]
fn a_code_block_stands_apart_from_prose_and_from_the_fence() {
    let rows = drawn("prose line\n```rust\nfn main() {}\n```", 60);
    let find = |want: &str| {
        rows.iter()
            .find(|(text, _)| text.trim() == want)
            .unwrap_or_else(|| panic!("row {want:?} must draw:\n{rows:#?}"))
            .1
    };
    let prose = find("prose line");
    let fence = find("```rust");
    let code = find("fn main() {}");
    assert_ne!(code.fg, prose.fg, "code is not prose");
    assert_ne!(fence.fg, code.fg, "the fence is not the code");
}

#[test]
fn stripping_happens_before_wrapping_so_the_measure_stays_honest() {
    // This is the ordering the phase split exists for. A heading's hashes are removed before
    // the text is wrapped, so no row is narrower than the wrap assumed, and no row overflows.
    let long = format!("## {}", "word ".repeat(40));
    let rows = drawn(&long, 40);
    let filled: Vec<&(String, ratatui::style::Style)> =
        rows.iter().filter(|(t, _)| !t.trim().is_empty()).collect();
    assert!(filled.len() > 3, "a long heading wraps");
    for (text, _) in &filled {
        assert!(
            text.chars().count() <= 40,
            "no row may exceed the width: {text:?}"
        );
        assert!(!text.contains('#'), "no hash survives: {text:?}");
    }
}

#[test]
fn an_escape_never_survives_markdown_styling() {
    // The scanner runs on text that `sanitize_block` already cleaned. It must not reintroduce
    // an escape, and it must not be fooled into treating one as markup.
    let hostile = "# \u{1b}[2Jheading\n```\n\u{1b}]52;c;aGk=\u{7}\n```";
    let rows = drawn(hostile, 60);
    let all = rows
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all.contains('\u{1b}'), "no escape reaches a cell: {all:?}");
    assert!(!all.contains("[2J"), "and no dropped parameters: {all:?}");
    assert!(!all.contains("52;c;"), "no OSC 52 payload: {all:?}");
}

#[test]
fn a_user_row_is_drawn_verbatim() {
    // rho draws what the user typed. A `#` they wrote stays a `#`.
    let mut state = TuiState::default();
    state.rows.push(Row::User {
        text: "# not a heading, just my text".to_string(),
    });
    let backend = TestBackend::new(60, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    let all: String = (0..24)
        .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("# not a heading, just my text"),
        "the user's hash survives:\n{all}"
    );
}

// ---- Three findings from the contract review, pinned as tests. ------------------

#[test]
fn fence_state_spans_the_whole_message_not_the_visible_window() {
    // The review called this out as high severity: if fence state were computed per visible
    // row, a code line whose opening fence had scrolled away would be reinterpreted as prose,
    // and a `#` or `-` inside code would be miscoloured. The scanner reads the whole message,
    // so a code line 200 lines below its fence is still code.
    let mut source = String::from("```rust\n");
    for index in 0..200 {
        source.push_str(&format!("# line {index} is code, not a heading\n"));
    }
    let scanned = scan_markdown(&source);
    assert_eq!(
        scanned.len(),
        202,
        "one fence, 200 code lines, one trailing blank"
    );
    for line in scanned.iter().skip(1).take(200) {
        assert_eq!(
            line.kind,
            MarkdownKind::CodeBlock,
            "every line after the fence stays code: {line:?}"
        );
    }
}

// `a_table_degrades_to_verbatim_text` lived here and is deliberately gone.
//
// It asserted that every table row stays plain text. That was right while rho drew no tables:
// the contract review warned that half a table styled is worse than none. rho now draws them,
// because a model emits a table in most answers, so the assertion contradicts the feature.
//
// The review's concern is still guarded, and by two tests in `tables.rs` rather than by not
// having the feature: `a_table_without_a_rule_row_stays_verbatim` and
// `a_table_inside_a_fence_stays_code`. A table only draws when it is unambiguous.

#[test]
fn an_alignment_row_is_not_mistaken_for_a_rule() {
    // `|---|---|` must not become a full-width rule through the middle of a table.
    assert_eq!(kinds("|---|---|"), vec![MarkdownKind::Text]);
    assert_eq!(kinds("| --- | --- |"), vec![MarkdownKind::Text]);
    // A one-column table. This case was added after a deliberate break showed the test
    // could not catch a scanner that trimmed the outer pipes before testing for a rule.
    assert_eq!(kinds("|---|"), vec![MarkdownKind::Text]);
    assert_eq!(kinds("|:---:|"), vec![MarkdownKind::Text]);
}

// A test was removed here, and the reason is worth keeping.
//
// The contract review warned that a row narrower than the terminal leaves the previous
// frame's cells behind. Two attempts to pin that failed, and the second failure showed why:
// the premise is false for a ratatui app. `Terminal::draw` resets its back buffer and emits a
// diff, so a shorter row already clears its own tail. Deleting the padding from the renderer
// changed nothing that any test could see, and a test that cannot fail is worse than none.
//
// So padding a row to the width is a consistency convention here, not a correctness guard,
// and this file no longer claims otherwise. The width drift the review really warned about is
// the ordering bug, and `stripping_happens_before_wrapping_so_the_measure_stays_honest`
// covers that one.

#[test]
fn the_scanner_only_deletes_and_inserts_known_glyphs() {
    // The contract review asked for this, and it is the right shape: pin the property, not an
    // example. `an_escape_never_survives_markdown_styling` proves one hostile string is safe.
    // This proves *why* the whole path is safe, which is that the scanner never synthesises a
    // character. It only drops a line's markup, keeps the rest, and inserts one of two fixed
    // glyphs. So it cannot introduce an escape, a control byte, or anything else.
    //
    // The corpus is deliberately hostile and includes raw escapes. In the product,
    // `sanitize_block` runs first, so the scanner never sees one. Feeding them here proves the
    // scanner is not the layer that would let one through.
    const INSERTED: [char; 2] = ['\u{2022}', '\u{2503}'];
    let corpus = [
        "# heading",
        "###### six",
        "#[derive(Debug)]",
        "- bullet",
        "* star bullet",
        "+ plus bullet",
        "  - nested bullet",
        "1. numbered",
        "12. numbered again",
        "1.2.3 version",
        "> quote",
        ">out.txt",
        "---",
        "***",
        "___",
        "|---|",
        "| a | b |",
        "```rust",
        "```",
        "plain prose",
        "",
        "\u{1b}[2J escape",
        "\u{1b}]52;c;aGk=\u{7}",
        "text\u{7}with\u{1}controls",
        "2 * 3 * 4",
        "wrap_block and snake_case",
        "--no-mouse",
        "# \u{1b}[31mred heading",
        "```\n# code not heading\n- code not bullet\n```",
        "# one\n\n- two\n> three\n---\n```\nfour\n```\nfive",
    ];
    for input in corpus {
        let input_chars: std::collections::HashSet<char> = input.chars().collect();
        for line in scan_markdown(input) {
            for ch in line.text.chars() {
                assert!(
                    input_chars.contains(&ch) || INSERTED.contains(&ch),
                    "the scanner invented {ch:?} from input {input:?} (row {:?}). It may only \
                     drop markup, keep text, and insert a bullet or a quote bar.",
                    line.text
                );
            }
        }
    }
}

#[test]
fn a_list_item_keeps_the_body_colour() {
    // Measured against pi, captured from a live run: pi colours the list marker and leaves the
    // item's text at the body colour. rho first coloured the whole item accent, which read as
    // louder than the prior art. Phase 1 cannot colour a glyph on its own, so the item takes
    // the body colour and the glyph carries the structure. See `D-markdown-line-level-first`.
    let rows = drawn("- an item\nplain prose", 60);
    let item = rows
        .iter()
        .find(|(text, _)| text.trim().starts_with('\u{2022}'))
        .expect("the item draws");
    let prose = rows
        .iter()
        .find(|(text, _)| text.trim() == "plain prose")
        .expect("the prose draws");
    assert_eq!(
        item.1.fg, prose.1.fg,
        "a list item reads as body text, not as an accent"
    );
}

#[test]
fn a_quote_draws_italic_and_quiet() {
    // A quote is someone else's voice, so it leans. pi italicises a blockquote and jcode does
    // not; the owner asked for italic. It stays quiet as well, because a quote is not the answer.
    let rows = drawn("> a quoted line\nplain prose", 60);
    let quote = rows
        .iter()
        .find(|(text, _)| text.contains("a quoted line"))
        .expect("the quote draws");
    let prose = rows
        .iter()
        .find(|(text, _)| text.trim() == "plain prose")
        .expect("the prose draws");
    assert!(
        quote.1.add_modifier.contains(Modifier::ITALIC),
        "a quote leans: {:?}",
        quote.1
    );
    assert!(
        !prose.1.add_modifier.contains(Modifier::ITALIC),
        "prose does not"
    );
    assert_ne!(
        quote.1.fg, prose.1.fg,
        "and a quote stays quieter than the answer"
    );
}
