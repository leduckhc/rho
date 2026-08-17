//! Terminal-safe text for MCP output.
//!
//! The filter itself lives in `rho-redact`, so there is exactly one implementation and
//! one test suite. This crate once carried its own copy, and the three copies in the
//! workspace had already drifted: this one dropped whole escape sequences while the
//! others replaced single characters. So the same hostile output rendered differently
//! depending on which path carried it. Decision D-026 records the consolidation.

/// Sanitise output from a server.
///
/// Output is untrusted, because somebody else wrote the server. An escape sequence in a
/// tool result must never reach the terminal.
pub fn sanitize_output(input: &str) -> String {
    rho_redact::sanitize_text(input)
}

/// Cap text at `max_bytes`, cutting on a character boundary.
///
/// A naive byte cut would split a multi-byte character and panic.
pub fn truncate_bytes(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[truncated: output over {max_bytes} bytes.]",
        &input[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_with_an_escape_sequence_is_sanitised() {
        assert_eq!(sanitize_output("red\u{1b}[31mtext"), "redtext");
    }

    #[test]
    fn long_output_is_capped_and_says_so() {
        let out = truncate_bytes(&"a".repeat(500), 100);
        assert!(out.contains("[truncated"), "{out}");
        assert!(out.len() < 200, "{out}");
    }

    #[test]
    fn a_cap_inside_a_multibyte_character_keeps_valid_text() {
        // A naive byte cut would split a character and panic.
        let out = truncate_bytes("日本語日本語", 4);
        assert!(out.contains("[truncated"), "{out}");
    }
}
