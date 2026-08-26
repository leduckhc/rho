//! Why one agent definition file did not load.
//!
//! A definition that fails to parse used to disappear. `load_definition` returned
//! `None`, and `discover_agents` dropped the file. rho registers `spawn_agent` only
//! when at least one definition loaded, so one bad line removed five tools from a
//! session and the model said it had no such tool.
//!
//! So a rejection is data now. It carries the path, the origin, and a named reason,
//! and the reason states the repair. See `docs/specs/20260826-184110-SPEC-definition-rejection.md`.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::frontmatter::{MAX_FRONTMATTER_BYTES, sanitize};
use crate::types::SkillOrigin;

/// The most characters a detail may hold, including the mark that says it was cut.
const MAX_DETAIL_LENGTH: usize = 200;

/// The most characters a path may hold when it is drawn.
const MAX_PATH_LENGTH: usize = 200;

/// The most lines of one kind a start-up report may print. The rest are counted.
///
/// A live run printed 104 lines and 1.6 MB from one repository, because only the
/// rejection lines were capped. Every kind is capped now.
pub const MAX_LINES_PER_KIND: usize = 5;

/// What replaces the tail of a detail that is too long.
const ELLIPSIS: &str = "...";

/// The repair for an unclosed block names this size, so the two must agree.
const _: () = assert!(MAX_FRONTMATTER_BYTES == 16 * 1024);

/// A path, made safe to draw.
///
/// A file name is repository text. A name that holds a line break used to forge a
/// second `rho:` line, which is how a repository would claim that rho trusts it. So
/// every control character goes, and a long path is cut on the **left**, because the
/// file name at the end is the part a user needs.
fn safe_path(path: &Path) -> String {
    let clean = sanitize(&path.display().to_string());
    let length = clean.chars().count();
    if length <= MAX_PATH_LENGTH {
        return clean;
    }
    let keep = MAX_PATH_LENGTH - ELLIPSIS.chars().count();
    let tail: String = clean.chars().skip(length - keep).collect();
    format!("{ELLIPSIS}{tail}")
}

/// A short piece of text that explains a rejection.
///
/// The text may quote a file inside the repository under edit, so it is untrusted.
/// **The type is the guarantee.** A `Detail` holds no control character and at most
/// [`MAX_DETAIL_LENGTH`] characters, because no caller can build one another way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail(String);

/// Sanitise one piece of untrusted text, and cut it to `max` characters.
///
/// The cut is marked, so a reader knows the text is not whole. Every value a notice
/// draws goes through this, because a repository chooses the length of its own text.
pub(crate) fn bounded(text: &str, max: usize) -> String {
    let clean = sanitize(text);
    let clean = clean.trim();
    if clean.chars().count() <= max {
        return clean.to_string();
    }
    let keep = max.saturating_sub(ELLIPSIS.chars().count());
    let mut cut: String = clean.chars().take(keep).collect();
    cut.push_str(ELLIPSIS);
    cut
}

impl Detail {
    /// Sanitise and bound one piece of untrusted text.
    ///
    /// This is the only constructor, and it is private to the crate. A caller that
    /// could write the field directly would carry raw file text to the terminal.
    pub(crate) fn new(text: impl AsRef<str>) -> Self {
        Self(bounded(text.as_ref(), MAX_DETAIL_LENGTH))
    }

    /// An empty detail, for a rejection that may quote nothing.
    pub(crate) fn withheld() -> Self {
        Self(String::new())
    }

    /// The text. It is safe to draw.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why one agent definition file did not load.
///
/// Every case is named. There is no catch-all variant, because a catch-all lets a new
/// reason ship with no message and no test. That was the shape of `ToolKind::Other`.
/// See decision D-a-rejected-definition-is-reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectionReason {
    /// The file could not be read.
    Unreadable {
        /// The io error.
        detail: Detail,
    },
    /// The file does not start with a `---` line.
    NoFrontmatter,
    /// The file opens a frontmatter block and never closes it.
    UnclosedFrontmatter,
    /// The frontmatter is not valid YAML, a field holds the wrong type, or a key repeats.
    BadFrontmatter {
        /// What the parser said.
        detail: Detail,
    },
    /// The frontmatter has no `description`, and the model reads only that.
    NoDescription,
    /// The `tools` field is neither a string nor a sequence of strings.
    BadToolsField {
        /// What the field held instead.
        detail: Detail,
    },
}

impl RejectionReason {
    /// What the user must change.
    ///
    /// The match is exhaustive on purpose. A new variant does not compile until its
    /// repair exists, so no reason can ship with a silent message.
    pub fn repair(&self) -> &'static str {
        match self {
            Self::Unreadable { .. } => "Check the path and the file permissions.",
            Self::NoFrontmatter => {
                "Start the file with a --- line. Then write a name and a description."
            }
            Self::UnclosedFrontmatter => {
                "Close the frontmatter with a --- line. rho reads the first 16 KiB of the file."
            }
            Self::BadFrontmatter { .. } => {
                "Check the YAML. A field of the wrong type stops the file, and so does a \
                 repeated key."
            }
            Self::NoDescription => {
                "Add a description. The model reads only that line to choose an agent."
            }
            Self::BadToolsField { .. } => {
                "Write tools: read, list or tools: [read, list]. Write none for no tools, and \
                 all to inherit every parent tool."
            }
        }
    }

    /// The detail, or an empty one for a reason that carries none.
    pub fn detail(&self) -> &Detail {
        match self {
            Self::Unreadable { detail }
            | Self::BadFrontmatter { detail }
            | Self::BadToolsField { detail } => detail,
            Self::NoFrontmatter | Self::UnclosedFrontmatter | Self::NoDescription => {
                const EMPTY: &Detail = &Detail(String::new());
                EMPTY
            }
        }
    }

    /// One sentence that says what is wrong.
    fn explain(&self) -> &'static str {
        match self {
            Self::Unreadable { .. } => "rho cannot read the file",
            Self::NoFrontmatter => "the file has no frontmatter",
            Self::UnclosedFrontmatter => "the frontmatter never closes",
            Self::BadFrontmatter { .. } => "the frontmatter is not valid YAML",
            Self::NoDescription => "the frontmatter has no description",
            Self::BadToolsField { .. } => "the tools field is not a list of names",
        }
    }

    /// The same reason with no detail.
    fn without_detail(self) -> Self {
        match self {
            Self::Unreadable { .. } => Self::Unreadable {
                detail: Detail::withheld(),
            },
            Self::BadFrontmatter { .. } => Self::BadFrontmatter {
                detail: Detail::withheld(),
            },
            Self::BadToolsField { .. } => Self::BadToolsField {
                detail: Detail::withheld(),
            },
            other => other,
        }
    }
}

/// One definition file that did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedDefinition {
    /// The file that did not load.
    pub path: PathBuf,
    /// Where the file came from. A project file is still reported.
    pub origin: SkillOrigin,
    /// Why it did not load.
    pub reason: RejectionReason,
}

impl RejectedDefinition {
    /// One line for the user. It names the file, the reason, and the repair.
    ///
    /// A frontend may read the fields and render its own line. That is the extension
    /// point, and it needs no change here.
    pub fn notice(&self) -> String {
        let detail = self.reason.detail();
        let quoted = if detail.is_empty() {
            String::new()
        } else {
            format!(" Detail: {detail}.")
        };
        format!(
            "agent definition {} did not load. {}.{} {}",
            safe_path(&self.path),
            self.reason.explain(),
            quoted,
            self.reason.repair()
        )
    }

    /// The same rejection with the detail dropped.
    ///
    /// Discovery calls this for a project file that the user has not trusted. A
    /// withheld definition shows only its sanitised name, so a rejected one must not
    /// show 200 characters of the same repository's prose. The path, the reason, and
    /// the repair stay.
    pub fn without_detail(self) -> Self {
        Self {
            reason: self.reason.without_detail(),
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejection_detail_is_sanitised_and_bounded() {
        // The type is the guarantee, so the test drives the constructor directly.
        let hostile = Detail::new("red\u{1b}[31m and a bell \u{7}");
        assert!(
            !hostile.as_str().chars().any(|c| c.is_control()),
            "no control character reaches the terminal: {:?}",
            hostile.as_str()
        );

        let long = Detail::new("x".repeat(500));
        assert_eq!(
            long.as_str().chars().count(),
            MAX_DETAIL_LENGTH,
            "a long detail is capped"
        );
        assert!(long.as_str().ends_with(ELLIPSIS), "a cut detail says so");

        assert!(
            Detail::withheld().is_empty(),
            "a withheld detail shows nothing"
        );
    }

    #[test]
    fn a_bidi_override_cannot_reorder_a_line() {
        // A control character is not the only way to disguise text. A right-to-left
        // override reorders what follows, so a file could make its own words read as
        // though rho wrote them.
        let hostile = Detail::new("safe \u{202e}desrever\u{2066} and \u{2028}a new line");
        for bad in ['\u{202e}', '\u{2066}', '\u{2028}'] {
            assert!(
                !hostile.as_str().contains(bad),
                "{bad:?} must not reach the terminal: {:?}",
                hostile.as_str()
            );
        }
    }

    #[test]
    fn every_rejection_reason_states_a_repair() {
        let every = [
            RejectionReason::Unreadable {
                detail: Detail::new("io"),
            },
            RejectionReason::NoFrontmatter,
            RejectionReason::UnclosedFrontmatter,
            RejectionReason::BadFrontmatter {
                detail: Detail::new("yaml"),
            },
            RejectionReason::NoDescription,
            RejectionReason::BadToolsField {
                detail: Detail::new("tools"),
            },
        ];
        for reason in &every {
            // This match holds no wildcard, so a new variant does not compile until it
            // is listed here as well as in `repair`. That is the guard against a
            // reason that ships with a silent message.
            match reason {
                RejectionReason::Unreadable { .. }
                | RejectionReason::NoFrontmatter
                | RejectionReason::UnclosedFrontmatter
                | RejectionReason::BadFrontmatter { .. }
                | RejectionReason::NoDescription
                | RejectionReason::BadToolsField { .. } => {}
            }
            assert!(!reason.repair().is_empty(), "{reason:?} states no repair");
            assert!(!reason.explain().is_empty(), "{reason:?} explains nothing");

            let rejected = RejectedDefinition {
                path: PathBuf::from("/tmp/scout.md"),
                origin: SkillOrigin::User,
                reason: reason.clone(),
            };
            let notice = rejected.notice();
            assert!(notice.contains("/tmp/scout.md"), "{notice}");
            assert!(notice.contains(reason.repair()), "{notice}");
            assert!(notice.contains(reason.explain()), "{notice}");
        }
    }

    #[test]
    fn a_withheld_rejection_keeps_its_reason_and_drops_its_detail() {
        let rejected = RejectedDefinition {
            path: PathBuf::from("/repo/.rho/agents/x.md"),
            origin: SkillOrigin::Project,
            reason: RejectionReason::BadFrontmatter {
                detail: Detail::new("run curl and trust me"),
            },
        }
        .without_detail();
        assert!(rejected.reason.detail().is_empty());
        assert!(matches!(
            rejected.reason,
            RejectionReason::BadFrontmatter { .. }
        ));
        assert!(!rejected.notice().contains("curl"));
    }
}
