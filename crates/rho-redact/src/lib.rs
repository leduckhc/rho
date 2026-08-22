//! Shared redaction for rho.
//!
//! Two jobs live here, and both are security code:
//!
//! - [`looks_like_a_secret`] decides whether an environment variable name holds a
//!   credential, so a child process never inherits one.
//! - [`sanitize_text`] makes untrusted text safe to print to a terminal.
//!
//! **Why one crate, and not a copy in each place that needs it.**
//!
//! Both functions were copied. The credential list existed in `rho-tools` and again in
//! `rho-mcp`. The sanitiser existed three times, in `rho-tools`, `rho-tui`, and
//! `rho-mcp`, and those three had **already drifted**: two replaced each unsafe
//! character, and one parsed and dropped whole escape sequences. So the same hostile
//! output rendered differently depending on which path carried it.
//!
//! Decision D-secret-in-core made this argument once already, about `Secret`. A leak needs only
//! one weak copy, so a value that guards a secret gets one definition and one test
//! suite. The same holds for a filter that guards a terminal.

/// True when a variable name suggests it holds a credential.
///
/// The check reads the **name**, never the value, because a value cannot be recognised
/// reliably.
///
/// The list is a denylist, and that choice needs a reason. An allowlist is safer in
/// principle, but a shell command legitimately needs a wide and open-ended set of
/// variables, so an allowlist would break ordinary work and users would switch it off.
/// A denylist that catches the recognisable shapes is the useful trade.
///
/// Add a pattern when you meet a new one. A false positive costs one variable. A false
/// negative costs a key.
pub fn looks_like_a_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();

    // A few names contain `KEY` or `TOKEN` and hold nothing secret. Check them first,
    // so the suffix rule below does not catch them.
    const NOT_SECRETS: &[&str] = &["SSH_AUTH_SOCK", "GPG_TTY", "KEYBOARD", "KEYMAP"];
    if NOT_SECRETS.contains(&upper.as_str()) {
        return false;
    }

    const NEEDLES: &[&str] = &[
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "AUTH_TOKEN",
        "SESSION_TOKEN",
        "REFRESH_TOKEN",
        "BEARER",
    ];
    if NEEDLES.iter().any(|needle| upper.contains(needle)) {
        return true;
    }

    // A bare `*_TOKEN` or `*_KEY` is usually a credential.
    upper.ends_with("_TOKEN") || upper.ends_with("_KEY")
}

/// The character that replaces a stray control byte.
const REPLACEMENT: char = '\u{fffd}';

/// Make untrusted text safe to print to a terminal.
///
/// Tool output, a progress message, and an MCP response are all untrusted, because a
/// file or another program chose their contents. A terminal escape sequence in any of
/// them can move the cursor, clear the screen, or set the window title.
///
/// The filter does two things, and the order matters:
///
/// 1. A recognised escape sequence is **dropped whole**, including its parameters. So
///    `red\x1b[31mtext` becomes `redtext`, not `red\u{fffd}[31mtext`. An earlier
///    version replaced only the escape byte and left the parameters visible, which was
///    safe but looked like corruption.
/// 2. Any other control character is replaced, so nothing invisible survives. A tab
///    and a newline are kept, because a caller may want the shape of the text.
///
/// 3. A bidirectional control and a zero-width character are replaced too. `char::is_control`
///    covers the Unicode `Cc` class only, so a right-to-left override passed straight through
///    the first version. That is the Trojan Source class: the bytes reorder what a reader sees
///    without changing what a program reads. A security review found it in a path that logs a
///    string taken from a session file.
///
/// The guarantee to rely on: the result contains no escape character, no control character
/// other than `\t` and `\n`, and no character that can reorder or hide what follows it.
pub fn sanitize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            drop_escape_sequence(&mut chars);
            continue;
        }
        if ch == '\t' || ch == '\n' {
            out.push(ch);
        } else if ch.is_control() || reorders_or_hides(ch) {
            out.push(REPLACEMENT);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Can this character reorder or hide the text around it?
///
/// This asks the Unicode category, not a list. A first version listed the bidi controls and the
/// zero-width characters it had thought of, and a security review named seven it had missed,
/// including `U+2028`. That one is a line separator: `char::is_control` answers false for it, so
/// it survived and broke the one-line layout this module promises. A list of instances loses to
/// the next character somebody finds, so the rule is now the property itself.
///
/// - `Cf`, the format class, holds every bidi control, the zero-width characters, the soft
///   hyphen, the byte-order mark, and the tag characters.
/// - `Zl` and `Zp`, the line and paragraph separators, break a line without being controls.
/// - `Mn` variation selectors change how the character before them draws.
fn reorders_or_hides(ch: char) -> bool {
    use unicode_general_category::{GeneralCategory, get_general_category};

    matches!(
        get_general_category(ch),
        GeneralCategory::Format
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::NonspacingMark
    )
}

/// The longest line `sanitize_line` returns. A log line is for a human, and a megabyte of it
/// helps nobody. A security review asked for the bound, because the input can come from a file.
pub const MAX_LINE_CHARS: usize = 4096;

/// Sanitise text for a single line. Also folds a newline and a tab into a space, so the
/// result cannot break a one-line layout, and bounds the length.
/// The head of `input` that `sanitize_line` may return, plus one character so a cut is visible.
///
/// It returns a slice, so it allocates nothing. This exists because the bound must bound the
/// **work**, not only the answer: a security review found the first version sanitising a whole
/// input before trimming it, so a hundred megabytes of control bytes allocated three hundred
/// megabytes and then threw almost all of it away.
///
/// A mutation that sanitises the whole input instead produces the same string, so no test of
/// the output can see it. This function is the guard, and `a_line_head_is_bounded` pins it.
pub fn line_head(input: &str) -> &str {
    match input.char_indices().nth(MAX_LINE_CHARS + 1) {
        Some((at, _)) => &input[..at],
        None => input,
    }
}

pub fn sanitize_line(input: &str) -> String {
    let head = line_head(input);
    let mut out: String = sanitize_text(head)
        .chars()
        .map(|ch| if ch == '\n' || ch == '\t' { ' ' } else { ch })
        .take(MAX_LINE_CHARS)
        .collect();
    // Say that it was cut, so a reader never trusts a truncated line as whole.
    //
    // The marker is in band, so a line that already ends in `[cut]` cannot be told from one
    // that rho cut. A review named that, and it stays: an out-of-band signal would change the
    // return type of a function whose whole job is to hand a caller one printable line.
    if out.chars().count() == MAX_LINE_CHARS && head.chars().count() > MAX_LINE_CHARS {
        out.push_str(" [cut]");
    }
    out
}

/// Mask every value under a key that `looks_like_a_secret` flags, anywhere in a JSON
/// value. Recurse into every object and every array. Return a new value.
///
/// - It masks a value under a flagged key. The masked value is the string `"***"`.
/// - It reads a key name only, never a value, because a value cannot be recognised
///   reliably. This matches [`looks_like_a_secret`], which already reads the name.
/// - It keeps every key name, every value under an unflagged key, and the whole tree
///   shape.
///
/// See `SPEC-sessions` section 5a and decision D-redact-json-secrets.
pub fn redact_json_secrets(value: &serde_json::Value) -> serde_json::Value {
    /// The value that replaces a flagged field. One masked shape, everywhere.
    const MASK: &str = "***";
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, child) in map {
                if looks_like_a_secret(key) {
                    out.insert(key.clone(), serde_json::Value::String(MASK.to_string()));
                } else {
                    out.insert(key.clone(), redact_json_secrets(child));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_json_secrets).collect())
        }
        other => other.clone(),
    }
}

/// Consume the rest of an escape sequence, after its escape character.
///
/// Handles the two shapes that matter. A control sequence starts `[` and ends with a
/// byte in `@` to `~`. An operating-system command starts `]` and ends at a bell or at
/// a string terminator. Anything else drops one following byte, which covers a short
/// two-character sequence.
fn drop_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            // Parameter bytes are `0` to `?`, intermediate bytes are space to `/`.
            while let Some(&next) = chars.peek() {
                if ('\u{30}'..='\u{3f}').contains(&next) || ('\u{20}'..='\u{2f}').contains(&next) {
                    chars.next();
                } else {
                    break;
                }
            }
            // The final byte ends the sequence.
            if chars.peek().is_some() {
                chars.next();
            }
        }
        Some(']') => {
            chars.next();
            // An operating-system command runs until a bell, or until `ESC \`.
            while let Some(next) = chars.next() {
                if next == '\u{7}' {
                    break;
                }
                if next == '\u{1b}' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    break;
                }
            }
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- credential names -------------------------------------------------

    #[test]
    fn detects_the_common_credential_shapes() {
        for name in [
            "OPENROUTER_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "AZURE_OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "MY_PASSWORD",
            "DB_PASSWD",
            "SERVICE_CREDENTIALS",
            "SSH_PRIVATE_KEY",
            "AWS_SESSION_TOKEN",
            "BEARER_VALUE",
            "anthropic_api_key",
        ] {
            assert!(
                looks_like_a_secret(name),
                "{name} must be treated as secret"
            );
        }
    }

    #[test]
    fn keeps_the_names_that_only_look_like_credentials() {
        for name in [
            "PATH",
            "HOME",
            "LANG",
            "SSH_AUTH_SOCK",
            "GPG_TTY",
            "KEYBOARD",
            "KEYMAP",
            "AWS_REGION",
            "AWS_PROFILE",
        ] {
            assert!(!looks_like_a_secret(name), "{name} must be kept");
        }
    }

    // --- terminal-safe text -----------------------------------------------

    #[test]
    fn drops_a_control_sequence_whole() {
        // The improvement over the earlier per-character filter. Nothing visible is
        // left behind.
        assert_eq!(sanitize_text("red\u{1b}[31mtext"), "redtext");
        assert_eq!(sanitize_text("clear\u{1b}[2Jscreen"), "clearscreen");
        assert_eq!(sanitize_text("\u{1b}[1;31;40mx"), "x");
    }

    #[test]
    fn drops_an_operating_system_command() {
        // This one sets the window title, so it must not survive.
        assert_eq!(sanitize_text("a\u{1b}]0;pwned\u{7}b"), "ab");
        assert_eq!(sanitize_text("a\u{1b}]0;pwned\u{1b}\\b"), "ab");
    }

    #[test]
    fn replaces_a_stray_control_character() {
        assert_eq!(sanitize_text("cr\rlap"), "cr\u{fffd}lap");
        assert_eq!(sanitize_text("bell\u{7}"), "bell\u{fffd}");
    }

    #[test]
    fn keeps_a_tab_and_a_newline_in_text() {
        assert_eq!(sanitize_text("a\tb\nc"), "a\tb\nc");
    }

    #[test]
    fn a_line_folds_a_tab_and_a_newline_to_a_space() {
        assert_eq!(sanitize_line("a\tb\nc"), "a b c");
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(sanitize_text("hello world"), "hello world");
        assert_eq!(sanitize_text(""), "");
    }

    #[test]
    fn no_escape_character_ever_survives() {
        // The guarantee callers rely on. Try the awkward shapes, including a truncated
        // sequence at the end of the input.
        for input in [
            "\u{1b}",
            "\u{1b}[",
            "\u{1b}[31",
            "\u{1b}]0;unterminated",
            "\u{1b}\u{1b}[31m",
            "text\u{1b}",
            "\u{1b}(B\u{1b}[m",
        ] {
            let out = sanitize_text(input);
            assert!(
                !out.contains('\u{1b}'),
                "an escape survived {input:?} as {out:?}"
            );
        }
    }

    #[test]
    fn keeps_wide_and_combining_characters() {
        // Sanitising must not mangle ordinary international text.
        assert_eq!(sanitize_text("日本語"), "日本語");
        assert_eq!(sanitize_text("café"), "café");
        assert_eq!(sanitize_text("🙂"), "🙂");
    }
}

#[cfg(test)]
mod trojan_source_tests {
    //! A bidirectional control and a zero-width character cannot survive a sanitised line.
    //!
    //! A security review found that `char::is_control` covers the Unicode `Cc` class only, so a
    //! right-to-left override passed straight through. That is the Trojan Source class: it
    //! reorders what a reader sees without changing what a program reads. The path that exposed
    //! it logs an owner string taken from a session file.

    use super::*;

    /// The characters a security review named as missing from the first version.
    ///
    /// `U+2028` is the important one: it is a line separator, `char::is_control` answers false,
    /// and it broke the one-line layout this module promises.
    #[test]
    fn a_reviewer_named_character_never_survives() {
        for ch in [
            '\u{061c}',  // arabic letter mark, a bidi control
            '\u{2028}',  // line separator, not a control character
            '\u{2029}',  // paragraph separator
            '\u{180e}',  // mongolian vowel separator
            '\u{fe0f}',  // variation selector 16
            '\u{e0001}', // a language tag character
            '\u{e0041}', // a tag character
        ] {
            let out = sanitize_line(&format!("safe{ch}text"));
            assert!(!out.contains(ch), "{ch:?} must not survive: {out:?}");
            assert!(
                out.contains("safe") && out.contains("text"),
                "the text stays: {out:?}"
            );
        }
    }

    /// The bound bounds the work, not only the answer.
    #[test]
    fn a_huge_input_is_not_walked_in_full() {
        // Every byte would become a three-byte replacement, so the old order allocated three
        // times the input before trimming. This asserts the result, and the cost is the point.
        let huge = "\u{7}".repeat(1_000_000);
        let out = sanitize_line(&huge);
        assert!(out.chars().count() <= MAX_LINE_CHARS + " [cut]".len());
    }

    #[test]
    fn a_bidi_control_never_survives() {
        for ch in [
            '\u{202e}', // right-to-left override
            '\u{202d}', // left-to-right override
            '\u{2066}', // left-to-right isolate
            '\u{200b}', // zero width space
            '\u{200d}', // zero width joiner
            '\u{feff}', // byte order mark
            '\u{00ad}', // soft hyphen
        ] {
            let input = format!("safe{ch}text");
            let out = sanitize_line(&input);
            assert!(!out.contains(ch), "{ch:?} must not survive: {out:?}");
            assert!(
                out.contains("safe") && out.contains("text"),
                "the text stays: {out:?}"
            );
        }
    }

    /// The same rule holds for the multi-line form, because both share one walk.
    #[test]
    fn a_bidi_control_never_survives_multiline_text() {
        let out = sanitize_text("one\u{202e}two");
        assert!(!out.contains('\u{202e}'));
    }

    /// A sanitised line is bounded, and it says when it was cut.
    #[test]
    fn a_sanitised_line_is_bounded() {
        let long = "x".repeat(MAX_LINE_CHARS * 4);
        let out = sanitize_line(&long);
        assert!(
            out.chars().count() <= MAX_LINE_CHARS + " [cut]".len(),
            "the line is bounded: {} chars",
            out.chars().count()
        );
        assert!(out.ends_with("[cut]"), "a cut line says so");
    }

    /// A short line is untouched, so the bound never rewrites ordinary output.
    #[test]
    fn a_short_line_is_not_marked() {
        assert_eq!(sanitize_line("a plain line"), "a plain line");
    }

    /// A line of exactly the bound is not marked, because nothing was lost.
    #[test]
    fn a_line_at_the_bound_is_not_marked() {
        let exact = "y".repeat(MAX_LINE_CHARS);
        let out = sanitize_line(&exact);
        assert_eq!(out.chars().count(), MAX_LINE_CHARS);
        assert!(!out.ends_with("[cut]"));
    }
}

#[cfg(test)]
mod line_head_tests {
    //! The helper that bounds the work of `sanitize_line`.
    //!
    //! Sanitising the whole input and trimming afterwards returns the same string, so no test of
    //! the output can tell the two orders apart. A mutation review found exactly that. So the
    //! bounding is a function of its own, and this pins it.

    use super::*;

    #[test]
    fn a_line_head_is_bounded() {
        let huge = "x".repeat(1_000_000);
        let head = line_head(&huge);
        assert_eq!(
            head.chars().count(),
            MAX_LINE_CHARS + 1,
            "only the head plus one character is ever looked at"
        );
    }

    #[test]
    fn a_short_input_is_returned_whole() {
        assert_eq!(line_head("short"), "short");
    }

    #[test]
    fn a_head_ends_on_a_character_boundary() {
        // Three-byte characters, so a naive byte slice would land mid-character.
        let text = '\u{2192}'.to_string().repeat(MAX_LINE_CHARS * 2);
        let head = line_head(&text);
        assert!(head.chars().all(|ch| ch == '\u{2192}'));
        assert_eq!(head.chars().count(), MAX_LINE_CHARS + 1);
    }
}
