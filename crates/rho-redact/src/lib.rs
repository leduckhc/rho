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
//! Decision D-014 made this argument once already, about `Secret`. A leak needs only
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
/// The guarantee to rely on: the result contains no escape character, and no control
/// character other than `\t` and `\n`.
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
        } else if ch.is_control() {
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
