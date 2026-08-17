//! A credential that cannot reach a log.
//!
//! See `SPEC-02` section 2 and decision D-014.
//!
//! This type lives in `rho-core` on purpose. It once existed twice, once in the
//! OpenRouter crate and once in the Azure crate, and the two copies already
//! differed. A leak needs only one weak copy. So the type that guards a secret
//! has exactly one definition and one test suite.

use std::fmt;

/// A credential that never prints itself.
///
/// The type has no `Display`. Its `Debug` prints a fixed mask. So a struct that
/// holds a `Secret` may derive `Debug` and still stay safe. That is the point:
/// the type redacts by construction, not by a filter that somebody must remember
/// to apply.
///
/// Read the value with [`Secret::expose`]. The name is deliberate. A reader of the
/// calling code can see where the value leaves its wrapper.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wrap a credential value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Read the value. Never log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when the credential is empty.
    ///
    /// An empty key reaches a provider and fails with a message that blames the
    /// service. Check this early and tell the user to set the key.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_prints_a_fixed_mask() {
        let secret = Secret::new("sk-live-abcdef123456");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
    }

    #[test]
    fn secret_debug_never_contains_the_value() {
        let secret = Secret::new("sk-live-abcdef123456");
        assert!(!format!("{secret:?}").contains("abcdef"));
    }

    #[test]
    fn secret_inside_a_derived_debug_struct_stays_masked() {
        // The real risk. A config struct derives `Debug`, somebody logs it, and the
        // key appears in a log file. The mask must survive that path.
        // The fields are read only through the derived `Debug`, which the lint does
        // not count as a read. That indirect read is the whole point of the test.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Config {
            base_url: String,
            api_key: Secret,
        }
        let config = Config {
            base_url: "https://example.test".to_string(),
            api_key: Secret::new("sk-live-abcdef123456"),
        };
        let printed = format!("{config:?}");
        assert!(
            printed.contains("example.test"),
            "the safe field still prints"
        );
        assert!(!printed.contains("abcdef"), "the key must not print");
        assert!(printed.contains("Secret(***)"));
    }

    #[test]
    fn secret_exposes_the_value_on_purpose() {
        assert_eq!(Secret::new("value").expose(), "value");
    }

    #[test]
    fn secret_reports_an_empty_value() {
        assert!(Secret::new("").is_empty());
        assert!(!Secret::new("k").is_empty());
    }
}
