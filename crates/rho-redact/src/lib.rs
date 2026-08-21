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
        // A connection string carries its password inline, so the variable name
        // never says "password". A security review proved `DATABASE_URL`,
        // `REDIS_URL`, and `MYSQL_PWD` all survived the scrub.
        "_PWD",
        "PGPASS",
        "DATABASE_URL",
        "DB_URL",
        "REDIS_URL",
        "AMQP_URL",
        "MONGO_URL",
        "MONGODB_URI",
        "CONNECTION_STRING",
    ];
    if NEEDLES.iter().any(|needle| upper.contains(needle)) {
        return true;
    }

    // A bare `*_TOKEN` or `*_KEY` is usually a credential.
    upper.ends_with("_TOKEN") || upper.ends_with("_KEY")
}

/// The character that replaces a stray control byte.
const REPLACEMENT: char = '\u{fffd}';

/// True for a character that is invisible or that reorders what follows it.
///
/// `char::is_control` is **false** for every one of these, so an earlier version of this filter let
/// them through. The guarantee said "no control character", which was true and was not the point.
///
/// Two families matter, and a security review proved both survive the older filter:
///
/// - **Bidirectional overrides and isolates**, `U+202A` to `U+202E` and `U+2066` to `U+2069`. These
///   are the Trojan Source trick: they reorder a line visually while the bytes say something else,
///   so a reviewer reads one thing and the machine runs another.
/// - **Zero-width and invisible marks**, including the byte order mark. Text can be hidden between
///   visible characters, so two lines that look identical are not.
///
/// **The two joiners are deliberately absent, and that is a correction.** `U+200C` and `U+200D` sit
/// inside the same numeric range and were swept in by the first version of this filter. They are not
/// spoofing tools, they are spelling: an emoji family is joined by `U+200D`, and Persian, Arabic and
/// several Indic scripts need `U+200C` to render a word correctly. Replacing them corrupted ordinary
/// text, and a second-opinion review caught it. Neither is a Trojan Source vector, which is the bidi
/// overrides and isolates below.
///
/// `U+2028` and `U+2029` are line and paragraph separators. They are one column wide, so they reach
/// a terminal cell, and some terminals treat them as a line break and shift the grid.
///
/// Every one is replaced rather than dropped, so nothing goes silently missing.
fn is_invisible_or_reordering(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}'                  // arabic letter mark
            | '\u{180e}'            // mongolian vowel separator
            | '\u{200b}'              // zero width space
            | '\u{200e}'              // left to right mark
            | '\u{200f}'              // right to left mark
            | '\u{202a}'..='\u{202e}' // bidi embedding and override
            | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{2028}'            // line separator
            | '\u{2029}'            // paragraph separator
            | '\u{feff}'            // byte order mark
    )
}

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
/// The guarantee to rely on: the result contains no escape character, no control character other
/// than `\t` and `\n`, and **no invisible or reordering character**. The last clause was added after
/// a security review proved that a bidirectional override and a line separator both survived the
/// first two. See `is_invisible_or_reordering`.
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
        } else if ch.is_control() || is_invisible_or_reordering(ch) {
            out.push(REPLACEMENT);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Sanitise text for a single line. Also folds a newline and a tab into a space, so the
/// result cannot break a one-line layout.
pub fn sanitize_line(input: &str) -> String {
    sanitize_text(input)
        .chars()
        .map(|ch| if ch == '\n' || ch == '\t' { ' ' } else { ch })
        .collect()
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
    #[test]
    fn a_connection_string_variable_is_a_secret() {
        // A security review proved these three survived the scrub. A connection
        // string holds its password inline, so the variable name never says
        // "password", and a name-based denylist misses it unless it is told.
        for name in [
            "DATABASE_URL",
            "REDIS_URL",
            "MYSQL_PWD",
            "PGPASSWORD",
            "MONGODB_URI",
            "AMQP_URL",
            "SPRING_DATASOURCE_CONNECTION_STRING",
        ] {
            assert!(
                super::looks_like_a_secret(name),
                "{name} carries a credential and must be scrubbed"
            );
        }
    }

    #[test]
    fn an_ordinary_variable_is_not_a_secret() {
        // The denylist must not swallow the environment. A scrub that hides PATH
        // breaks every command.
        for name in [
            "PATH",
            "HOME",
            "LANG",
            "TERM",
            "CARGO_TARGET_DIR",
            "RHO_MODEL",
        ] {
            assert!(
                !super::looks_like_a_secret(name),
                "{name} is not a credential and must survive"
            );
        }
    }

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
mod invisible_tests {
    use super::{sanitize_line, sanitize_text};

    /// Found by a security review. `char::is_control` is false for all of these, so the filter let
    /// them through while its guarantee said "no control character". True, and not the point.
    #[test]
    fn a_bidi_override_never_survives() {
        // The Trojan Source trick: reorder a line visually while the bytes say something else.
        for ch in [
            '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}',
        ] {
            let text = format!("safe{ch}hidden");
            let clean = sanitize_text(&text);
            assert!(
                !clean.contains(ch),
                "{ch:?} (U+{:04X}) survived: {clean:?}",
                ch as u32
            );
        }
    }

    #[test]
    fn a_zero_width_character_never_survives() {
        // Hidden text between visible characters makes two identical-looking lines differ.
        //
        // `U+200C` and `U+200D` were in this list and are deliberately gone. They are the zero width
        // non-joiner and joiner, and they are spelling rather than spoofing: an emoji family is joined
        // by `U+200D`, and Persian, Arabic and several Indic scripts need `U+200C`. Rejecting them
        // corrupted ordinary text. `a_zero_width_joiner_survives_because_it_is_spelling` covers the
        // other side, so neither behaviour can drift without a test failing.
        for ch in [
            '\u{200b}', '\u{200e}', '\u{200f}', '\u{feff}', '\u{2060}', '\u{061c}', '\u{180e}',
        ] {
            let clean = sanitize_text(&format!("a{ch}b"));
            assert!(
                !clean.contains(ch),
                "{ch:?} (U+{:04X}) survived: {clean:?}",
                ch as u32
            );
        }
    }

    #[test]
    fn a_line_separator_never_survives() {
        // U+2028 and U+2029 are one column wide, so they reach a terminal cell, and some terminals
        // treat them as a line break and shift the grid.
        for ch in ['\u{2028}', '\u{2029}'] {
            let clean = sanitize_text(&format!("AAA{ch}BBB"));
            assert!(!clean.contains(ch), "{ch:?} survived: {clean:?}");
        }
    }

    #[test]
    fn an_ordinary_character_is_untouched() {
        // The filter must not become a blunt instrument. Accents, CJK, emoji, and a combining mark
        // all stay, because a model writes them for a reason.
        let text = "caf\u{e9} na\u{ef}ve \u{4e2d}\u{6587} \u{1f600} e\u{301} tab\there";
        assert_eq!(sanitize_text(text), text);
    }

    #[test]
    fn the_replacement_marks_the_removal() {
        // Replaced, not dropped, so nothing goes silently missing.
        let clean = sanitize_line("a\u{202e}b");
        assert!(
            clean.contains('\u{fffd}'),
            "the removal is marked: {clean:?}"
        );
    }
}

#[cfg(test)]
mod joiner_tests {
    use super::sanitize_text;

    /// A regression the controller introduced and a second-opinion review caught.
    ///
    /// The filter that closed the Trojan Source hole took `U+200B` to `U+200F` as one range, which
    /// swept in the zero width joiner and non-joiner. Those two are not spoofing tools, they are
    /// **spelling**: an emoji family is joined by `U+200D`, and Persian, Arabic, and several Indic
    /// scripts need `U+200C` to render words correctly.
    ///
    /// The first version of this test used a single emoji, which has no joiner, so it passed while a
    /// family emoji and a Persian word were being corrupted.
    #[test]
    fn a_zero_width_joiner_survives_because_it_is_spelling() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(
            sanitize_text(family),
            family,
            "an emoji family keeps its joiners"
        );
        let persian = "\u{645}\u{6cc}\u{200c}\u{62e}\u{648}\u{627}\u{647}\u{645}";
        assert_eq!(
            sanitize_text(persian),
            persian,
            "a Persian word keeps its non-joiner"
        );
    }

    /// The hole the joiners were swept up by is still closed.
    #[test]
    fn the_spoofing_characters_are_still_rejected() {
        // A bidirectional override or isolate reorders a line visually. A zero width space hides
        // between characters. A directional mark flips a run. None of these is spelling.
        for ch in [
            '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}', '\u{200b}', '\u{200e}', '\u{200f}', '\u{feff}', '\u{2028}',
            '\u{2029}',
        ] {
            let text = format!("a{ch}b");
            assert!(
                !sanitize_text(&text).contains(ch),
                "U+{:04X} must not survive",
                ch as u32
            );
        }
    }
}
