//! Strip a leading `<thinking>` tag from an assistant text stream.
//!
//! Some models write their reasoning as `<thinking>...</thinking>` inside ordinary
//! text, because rho never asked them for a structured reasoning block. rho then drew
//! that text as the answer. This module reclassifies a **leading** tag pair as
//! reasoning, so a frontend can draw it dimmed. See
//! `SPEC-reasoning-across-providers` section 3 "Two" and section 6 rule 7.
//!
//! The rule is narrow on purpose:
//!
//! - A tag is stripped only from the **start** of a message, and only when the opening
//!   tag is the first non-space text. A tag in the middle of an answer is prose about
//!   tags, and it stays text.
//! - The accepted names are `<thinking>` and `<think>`. The tag name match ignores
//!   case, so `<Thinking>` also opens a block. See decision D-thinking-tag-case-insensitive.
//! - A self-closing `<thinking/>` is an empty reasoning block, not an opening tag. It
//!   never swallows the text that follows it.
//! - Text inside the pair becomes `Reasoning`. Text after the closing tag becomes `Text`.
//! - An unclosed tag runs to the end of the message, because a truncated stream must not
//!   lose the answer. It never runs past the message into a tool call, because a tool
//!   call is a separate event this splitter never sees.
//!
//! This runs on a **stream**, so the opening tag may arrive split across deltas. The
//! splitter holds a small **lead buffer** and refuses to classify text until it can
//! decide. The buffer invariant: while the state is `Lead` or `Inside`, the buffer holds
//! only bytes whose classification is not yet decided. In `Lead` the buffer never grows
//! past the longest opening tag plus its leading whitespace, because the first byte that
//! rules a tag out flushes the buffer as text. In `Inside` the buffer holds back only a
//! suffix that is a prefix of a closing tag.
//!
//! **A known limit.** The splitter cannot tell a model's own reasoning tag from an answer
//! that a user asked the model to print with a literal `<thinking>` tag. Both look the
//! same in the stream. rho strips the leading tag either way, because that fixes the
//! reported bug, and it never deletes the text: the text is kept as reasoning and stays
//! visible in the `full` display mode. So the worst case is a literal tag drawn dimmed,
//! not a lost answer. See the report and section 3 "Two".

/// One classified piece of assistant output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThinkingPiece {
    /// Reasoning text, from inside a leading tag pair.
    Reasoning(String),
    /// Ordinary answer text.
    Text(String),
}

/// The opening tags, lowercase. The match lowercases the candidate first.
const OPEN_TAGS: [&str; 2] = ["<thinking>", "<think>"];
/// The self-closing tags, lowercase. Each is an empty reasoning block.
const SELF_TAGS: [&str; 2] = ["<thinking/>", "<think/>"];
/// The closing tags, lowercase.
const CLOSE_TAGS: [&str; 2] = ["</thinking>", "</think>"];

/// The state the splitter carries between deltas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SplitState {
    /// At the message start, deciding whether a leading tag opens.
    #[default]
    Lead,
    /// Inside a confirmed tag, seeking the close.
    Inside,
    /// Past any tag. Every later byte is answer text.
    Text,
}

/// A stateful stripper for one assistant text block.
///
/// Feed it each text delta with [`push`](Self::push). Call [`finish`](Self::finish) when
/// the text block ends, so a held-back lead or an unclosed tag flushes. Build a fresh
/// splitter for each new text block, because the leading rule is per block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThinkingSplitter {
    state: SplitState,
    /// The undecided bytes. See the buffer invariant in the module comment.
    buffer: String,
}

impl ThinkingSplitter {
    /// Build a splitter for a new text block.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one text delta. Return the pieces that are now decided.
    pub fn push(&mut self, delta: &str) -> Vec<ThinkingPiece> {
        self.buffer.push_str(delta);
        let mut out = Vec::new();
        self.drive(&mut out);
        out
    }

    /// Flush the held-back buffer at the end of the text block.
    ///
    /// A lead that never became a tag flushes as text, so no text is lost. An unclosed
    /// tag flushes as reasoning, because it ran to the end of the message.
    pub fn finish(&mut self) -> Vec<ThinkingPiece> {
        let mut out = Vec::new();
        match self.state {
            SplitState::Lead => {
                if !self.buffer.is_empty() {
                    out.push(ThinkingPiece::Text(std::mem::take(&mut self.buffer)));
                }
            }
            SplitState::Inside => {
                if !self.buffer.is_empty() {
                    out.push(ThinkingPiece::Reasoning(std::mem::take(&mut self.buffer)));
                }
            }
            SplitState::Text => {
                if !self.buffer.is_empty() {
                    out.push(ThinkingPiece::Text(std::mem::take(&mut self.buffer)));
                }
            }
        }
        *self = Self::default();
        out
    }

    /// Drive the state machine until it needs more input.
    fn drive(&mut self, out: &mut Vec<ThinkingPiece>) {
        loop {
            let made_progress = match self.state {
                SplitState::Lead => self.step_lead(out),
                SplitState::Inside => self.step_inside(out),
                SplitState::Text => {
                    if !self.buffer.is_empty() {
                        out.push(ThinkingPiece::Text(std::mem::take(&mut self.buffer)));
                    }
                    false
                }
            };
            if !made_progress {
                return;
            }
        }
    }

    /// One step in the `Lead` state. Return true when it transitioned, so `drive` loops.
    fn step_lead(&mut self, out: &mut Vec<ThinkingPiece>) -> bool {
        // Find the first non-space byte. Whitespace before a tag is not the answer, so a
        // leading tag still counts as the first non-space text.
        let Some(first) = self.buffer.find(|c: char| !c.is_whitespace()) else {
            // Still all whitespace, or empty. Wait for more.
            return false;
        };
        let rest = &self.buffer[first..];
        let lower = rest.to_ascii_lowercase();

        // A full opening tag. Discard the whitespace and the tag, and go inside.
        if let Some(open) = OPEN_TAGS.iter().find(|tag| lower.starts_with(**tag)) {
            let after = rest[open.len()..].to_string();
            self.buffer = after;
            self.state = SplitState::Inside;
            return true;
        }

        // A self-closing tag. An empty reasoning block, then the rest is text.
        if let Some(tag) = SELF_TAGS.iter().find(|tag| lower.starts_with(**tag)) {
            out.push(ThinkingPiece::Reasoning(String::new()));
            let after = rest[tag.len()..].to_string();
            self.buffer = after;
            self.state = SplitState::Text;
            return true;
        }

        // A prefix of a tag that is still forming. Wait for more bytes.
        let could_open = OPEN_TAGS
            .iter()
            .chain(SELF_TAGS.iter())
            .any(|tag| tag.starts_with(&lower));
        if could_open {
            return false;
        }

        // The first non-space text is not a tag. The whole buffer is answer text.
        out.push(ThinkingPiece::Text(std::mem::take(&mut self.buffer)));
        self.state = SplitState::Text;
        true
    }

    /// One step in the `Inside` state. Return true when the closing tag was found.
    fn step_inside(&mut self, out: &mut Vec<ThinkingPiece>) -> bool {
        let lower = self.buffer.to_ascii_lowercase();

        // The earliest full closing tag, if any.
        let close = CLOSE_TAGS
            .iter()
            .filter_map(|tag| lower.find(tag).map(|at| (at, tag.len())))
            .min_by_key(|(at, _)| *at);

        if let Some((at, len)) = close {
            if at > 0 {
                out.push(ThinkingPiece::Reasoning(self.buffer[..at].to_string()));
            }
            let after = self.buffer[at + len..].to_string();
            self.buffer = after;
            self.state = SplitState::Text;
            return true;
        }

        // No full close. Hold back the longest suffix that is a prefix of a closing tag,
        // so a close split across deltas is not missed. The held-back suffix is ASCII, so
        // the split point is always a character boundary.
        let hold = self.closing_prefix_suffix_len(&lower);
        let emit_to = self.buffer.len() - hold;
        if emit_to > 0 {
            out.push(ThinkingPiece::Reasoning(self.buffer[..emit_to].to_string()));
            self.buffer.drain(..emit_to);
        }
        false
    }

    /// The length, in bytes, of the longest suffix of `lower` that is a proper prefix of a
    /// closing tag. Zero when no suffix can begin a closing tag.
    fn closing_prefix_suffix_len(&self, lower: &str) -> usize {
        let mut best = 0;
        for tag in CLOSE_TAGS {
            // A proper prefix only: a full match is handled by the caller.
            for len in 1..tag.len() {
                if lower.ends_with(&tag[..len]) {
                    best = best.max(len);
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed one delta and finish, collecting every piece.
    fn split_once(input: &str) -> Vec<ThinkingPiece> {
        let mut splitter = ThinkingSplitter::new();
        let mut out = splitter.push(input);
        out.extend(splitter.finish());
        out
    }

    fn reasoning(pieces: &[ThinkingPiece]) -> String {
        pieces
            .iter()
            .filter_map(|p| match p {
                ThinkingPiece::Reasoning(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn text(pieces: &[ThinkingPiece]) -> String {
        pieces
            .iter()
            .filter_map(|p| match p {
                ThinkingPiece::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_leading_thinking_tag_becomes_reasoning() {
        let pieces = split_once("<thinking>a</thinking>b");
        assert_eq!(reasoning(&pieces), "a");
        assert_eq!(text(&pieces), "b");
    }

    #[test]
    fn a_leading_think_tag_becomes_reasoning() {
        let pieces = split_once("<think>a</think>b");
        assert_eq!(reasoning(&pieces), "a");
        assert_eq!(text(&pieces), "b");
    }

    #[test]
    fn a_tag_in_the_middle_stays_text() {
        let input = "here is a <thinking> tag";
        let pieces = split_once(input);
        assert_eq!(reasoning(&pieces), "");
        assert_eq!(text(&pieces), input, "a middle tag must survive whole");
    }

    #[test]
    fn an_unclosed_tag_ends_at_the_message_end() {
        let pieces = split_once("<thinking>still thinking");
        assert_eq!(reasoning(&pieces), "still thinking");
        assert_eq!(text(&pieces), "", "a truncated stream loses no text");
    }

    #[test]
    fn an_opening_tag_split_across_three_deltas() {
        let mut splitter = ThinkingSplitter::new();
        let mut pieces = splitter.push("<thi");
        pieces.extend(splitter.push("nk"));
        pieces.extend(splitter.push("ing>x</think>y"));
        pieces.extend(splitter.finish());
        assert_eq!(reasoning(&pieces), "x");
        assert_eq!(text(&pieces), "y");
    }

    #[test]
    fn a_mixed_case_tag_opens_a_block() {
        let pieces = split_once("<Thinking>a</Thinking>b");
        assert_eq!(reasoning(&pieces), "a");
        assert_eq!(text(&pieces), "b");
    }

    #[test]
    fn a_self_closing_tag_is_an_empty_block_and_keeps_the_rest() {
        let pieces = split_once("<thinking/>done");
        assert_eq!(
            reasoning(&pieces),
            "",
            "a self-closing tag is empty reasoning"
        );
        assert_eq!(text(&pieces), "done", "it must not swallow the answer");
    }

    #[test]
    fn an_unclosed_tag_before_a_tool_call_keeps_only_the_reasoning() {
        // The splitter sees only the text block. `finish` runs when the text block ends,
        // before a tool call. The reasoning flushes, and no answer is swallowed, because
        // there is no answer text in an unclosed block.
        let mut splitter = ThinkingSplitter::new();
        let mut pieces = splitter.push("<thinking>let me use a tool");
        pieces.extend(splitter.finish());
        assert_eq!(reasoning(&pieces), "let me use a tool");
        assert_eq!(text(&pieces), "");
    }

    #[test]
    fn a_closing_tag_split_across_two_deltas() {
        let mut splitter = ThinkingSplitter::new();
        let mut pieces = splitter.push("<thinking>abc</thin");
        pieces.extend(splitter.push("king>tail"));
        pieces.extend(splitter.finish());
        assert_eq!(reasoning(&pieces), "abc");
        assert_eq!(text(&pieces), "tail");
    }

    #[test]
    fn plain_text_with_no_tag_stays_text() {
        let pieces = split_once("just a normal answer");
        assert_eq!(text(&pieces), "just a normal answer");
        assert_eq!(reasoning(&pieces), "");
    }

    #[test]
    fn a_multibyte_reasoning_body_is_not_corrupted() {
        let pieces = split_once("<thinking>café ☕ 日本</thinking>ok");
        assert_eq!(reasoning(&pieces), "café ☕ 日本");
        assert_eq!(text(&pieces), "ok");
    }
}
