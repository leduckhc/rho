//! Named handles and aliases: a second address for a child.
//!
//! The id stays the identity, unique per process. A handle is a name derived from the
//! agent name, unique inside one tree, so a model can say `explore-2` instead of
//! holding a number. An alias is a name the caller chooses. See
//! `SPEC-subagent-slots-handles-grace` section 3 and decision D-handle-is-a-second-address.
//!
//! Everything here is a pure function or a plain type. The registry owns the tables,
//! because a name must be derived inside the same lock as the registration. See
//! decision D-one-registry-state-lock.

use serde::de::{self, Deserialize, Deserializer, Visitor};

use crate::subagent::tree::AgentId;

/// The longest alias, in characters. The same rule as an agent name.
///
/// Counted in characters and not bytes, so an emoji costs one. A model writes an alias
/// and rho stores it, so the bound is rho's, not the model's.
pub const MAX_ALIAS_LENGTH: usize = 64;

/// A model-facing reference to a child: an id, or a name.
///
/// A JSON number reads as `Id` and a JSON string reads as `Name`. A digits-only name is
/// resolved as an id, so a model that sends `"42"` reaches the same child as one that
/// sends `42`. See decision D-agent-ref-accepts-id-or-name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRef {
    /// The id a spawn returned.
    Id(u64),
    /// A derived handle, or an alias.
    Name(String),
}

impl AgentRef {
    /// The text every refusal shares, so two tools cannot teach two things.
    fn refusal(arrived: &str) -> String {
        format!(
            "the id must be a subagent id, such as 7, or a handle, such as \"explore-2\". \
             This call sent {arrived}. Call agent_status with no argument to list what is \
             running."
        )
    }
}

impl std::fmt::Display for AgentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentRef::Id(id) => write!(f, "{id}"),
            AgentRef::Name(name) => write!(f, "{name}"),
        }
    }
}

/// The `Deserialize` impl is written by hand, not derived with `untagged`.
///
/// An untagged enum answers a boolean, a float, a negative number, `null`, or an object
/// with serde's own message, "data did not match any variant". That message teaches
/// nothing, and a refusal must teach. This impl names both accepted shapes and shows
/// what arrived.
impl<'de> Deserialize<'de> for AgentRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(AgentRefVisitor)
    }
}

struct AgentRefVisitor;

impl<'de> Visitor<'de> for AgentRefVisitor {
    type Value = AgentRef;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a subagent id, such as 7, or a handle, such as \"explore-2\"")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<AgentRef, E> {
        Ok(AgentRef::Id(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<AgentRef, E> {
        // A negative number is no id. The counter starts at zero and only grows.
        match u64::try_from(value) {
            Ok(id) => Ok(AgentRef::Id(id)),
            Err(_) => Err(E::custom(AgentRef::refusal(&value.to_string()))),
        }
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<AgentRef, E> {
        // serde reads a number past `u64` as a float, so this arm also refuses
        // 18446744073709551616. A truncation would address a child that does not exist.
        Err(E::custom(AgentRef::refusal(&value.to_string())))
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<AgentRef, E> {
        Err(E::custom(AgentRef::refusal(&value.to_string())))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<AgentRef, E> {
        // An empty string parses. It matches no child, which is an ordinary
        // not-found result and not a fault.
        Ok(AgentRef::Name(value.to_string()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<AgentRef, E> {
        Err(E::custom(AgentRef::refusal("null")))
    }

    fn visit_none<E: de::Error>(self) -> Result<AgentRef, E> {
        Err(E::custom(AgentRef::refusal("null")))
    }

    fn visit_map<A: de::MapAccess<'de>>(self, _map: A) -> Result<AgentRef, A::Error> {
        Err(de::Error::custom(AgentRef::refusal("an object")))
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, _seq: A) -> Result<AgentRef, A::Error> {
        Err(de::Error::custom(AgentRef::refusal("a list")))
    }
}

/// Why an alias was refused. Every case names what to do instead.
///
/// A refused alias never fails the spawn, because the child is already admitted and the
/// work matters more than the label. See `SPEC-subagent-slots-handles-grace` section 3.2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasError {
    /// A derived handle already holds this name in this tree.
    ShadowsHandle { name: String },
    /// Another alias already holds this name in this tree.
    Taken { name: String },
    /// The id names no child of this caller.
    Unknown { id: AgentId },
    /// The name is longer than [`MAX_ALIAS_LENGTH`].
    TooLong { limit: usize, length: usize },
    /// The name holds a character an alias may not hold.
    NotPrintable { name: String },
    /// The name is only digits, so an id would always win.
    DigitsOnly { name: String },
}

impl std::fmt::Display for AliasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AliasError::ShadowsHandle { name } => write!(
                f,
                "the name \"{name}\" is already a subagent's own handle, so it cannot be an \
                 alias. Choose another name, or address that child by \"{name}\"."
            ),
            AliasError::Taken { name } => write!(
                f,
                "the name \"{name}\" already names another subagent in this session. Choose \
                 another name."
            ),
            AliasError::Unknown { id } => write!(
                f,
                "no subagent with id {} belongs to this session, so it cannot be named. It \
                 may have finished, or it may belong to another session.",
                id.0
            ),
            AliasError::TooLong { limit, length } => write!(
                f,
                "a name holds at most {limit} characters, and this one holds {length}. Send a \
                 shorter name."
            ),
            AliasError::NotPrintable { name } => write!(
                f,
                "a name holds no control character and no line break, and \"{}\" does. Send a \
                 plain name.",
                name.escape_debug()
            ),
            AliasError::DigitsOnly { name } => write!(
                f,
                "the name \"{name}\" is only digits, and digits always read as a subagent id, \
                 so the name could never reach the child. Send a name with a letter in it."
            ),
        }
    }
}

impl std::error::Error for AliasError {}

/// True when every character is an ASCII digit, and there is at least one.
///
/// A digits-only name resolves as an id, so it can never resolve as a handle. That is
/// why an alias may not be digits only.
pub(crate) fn is_digits_only(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())
}

/// Check an alias the model wrote, before anything stores it.
///
/// The order is deliberate: length first, then the characters, then the digits rule. A
/// long name full of control characters is refused for its length, which is the fault
/// the caller should fix first.
pub(crate) fn validate_alias(alias: &str) -> Result<(), AliasError> {
    let length = alias.chars().count();
    if length > MAX_ALIAS_LENGTH {
        return Err(AliasError::TooLong {
            limit: MAX_ALIAS_LENGTH,
            length,
        });
    }
    // A space is fine. A newline, a bell, or an escape is not: an alias is echoed into
    // the parent's context beside rho's own lines, and a line break would forge one.
    if alias.chars().any(|c| c != ' ' && is_unprintable(c)) {
        return Err(AliasError::NotPrintable {
            name: alias.to_string(),
        });
    }
    if is_digits_only(alias) {
        return Err(AliasError::DigitsOnly {
            name: alias.to_string(),
        });
    }
    Ok(())
}

/// True when a character must never reach a terminal or a model's context.
fn is_unprintable(c: char) -> bool {
    c.is_control() || c == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&c)
}

/// Pick the handle for a new child, given every name already bound in its tree.
///
/// The first child of an agent takes the agent's own name. A collision is numbered from
/// two. `taken` must hold every name in the tree, live, queued, and remembered, or one
/// name would bind two children and `resolve` would pick one of them in silence.
///
/// The loop is bounded by `taken.len() + 2`, because at most `taken.len()` names are
/// unavailable. So it always terminates, whatever the model asks for.
pub(crate) fn derive_handle(taken: &std::collections::HashSet<String>, agent: &str) -> String {
    if !taken.contains(agent) {
        return agent.to_string();
    }
    for suffix in 2..=(taken.len() + 2) {
        let candidate = format!("{agent}-{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    // Unreachable: the range holds more candidates than `taken` can block.
    unreachable!("a free handle exists, because the search range exceeds the taken set")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(names: &[&str]) -> std::collections::HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_free_name_is_taken_as_it_stands() {
        assert_eq!(derive_handle(&taken(&[]), "explore"), "explore");
    }

    #[test]
    fn a_collision_counts_from_two_and_skips_what_is_held() {
        assert_eq!(derive_handle(&taken(&["explore"]), "explore"), "explore-2");
        assert_eq!(
            derive_handle(&taken(&["explore", "explore-2"]), "explore"),
            "explore-3"
        );
        // A hole is filled, so a finished `explore-2` frees exactly its own name.
        assert_eq!(
            derive_handle(&taken(&["explore", "explore-3"]), "explore"),
            "explore-2"
        );
    }

    #[test]
    fn the_search_terminates_when_every_low_number_is_held() {
        let held: Vec<String> = std::iter::once("explore".to_string())
            .chain((2..=50).map(|n| format!("explore-{n}")))
            .collect();
        let set: std::collections::HashSet<String> = held.into_iter().collect();
        assert_eq!(derive_handle(&set, "explore"), "explore-51");
    }

    #[test]
    fn a_digits_only_name_is_recognised() {
        assert!(is_digits_only("42"));
        assert!(!is_digits_only(""));
        assert!(!is_digits_only("4a"));
        assert!(!is_digits_only("-4"));
    }

    #[test]
    fn an_alias_is_bounded_printable_and_never_digits() {
        assert!(validate_alias("auth-audit").is_ok());
        assert!(validate_alias("a name with spaces").is_ok());
        assert_eq!(
            validate_alias(&"x".repeat(MAX_ALIAS_LENGTH + 1)),
            Err(AliasError::TooLong {
                limit: MAX_ALIAS_LENGTH,
                length: MAX_ALIAS_LENGTH + 1
            })
        );
        // The cap counts characters, so 64 emoji pass.
        assert!(validate_alias(&"\u{1f600}".repeat(MAX_ALIAS_LENGTH)).is_ok());
        assert!(matches!(
            validate_alias("two\nlines"),
            Err(AliasError::NotPrintable { .. })
        ));
        assert!(matches!(
            validate_alias("7"),
            Err(AliasError::DigitsOnly { .. })
        ));
    }
}
