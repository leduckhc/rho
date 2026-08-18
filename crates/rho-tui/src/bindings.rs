//! Discovery: the binding table, the help screen, and the slash commands.
//!
//! The binding table is the single source of truth. The help screen is generated
//! from it, so the two can never drift. The slash list filters as the user types,
//! and an unknown command reports rather than doing nothing. See
//! `SPEC-tui-experience` section 9 and `docs/tui-design.md` section 10.

/// One key binding. The binding table is the single source of truth.
pub struct Binding {
    /// The key or key pair, for example `alt+enter`.
    pub keys: &'static str,
    /// The one-line summary shown on the help screen.
    pub summary: &'static str,
}

/// The whole binding table.
pub fn bindings() -> &'static [Binding] {
    todo!("bindings is unimplemented in the red stage")
}

/// The help rows, generated from the binding table, so the two cannot drift.
pub fn help_rows() -> Vec<String> {
    todo!("help_rows is unimplemented in the red stage")
}

/// One slash command shown in the slash list.
pub struct SlashCommand {
    /// The command name, with its leading slash, for example `/guide`.
    pub name: &'static str,
    /// The one-line summary shown beside it in the list.
    pub summary: &'static str,
}

/// The whole slash-command list, in display order.
pub fn slash_commands() -> &'static [SlashCommand] {
    todo!("slash_commands is unimplemented in the red stage")
}

/// The slash commands whose name starts with the typed query, so the list filters
/// as the user types. The query carries its leading slash, for example `/g`.
pub fn filter_slash_commands(_query: &str) -> Vec<&'static SlashCommand> {
    todo!("filter_slash_commands is unimplemented in the red stage")
}

/// What running a typed slash command does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashOutcome {
    /// The command matched. The name is the resolved command.
    Run(String),
    /// The command did not match any known command. The message reports it, so an
    /// unknown command never silently does nothing.
    Unknown(String),
}

/// Resolve a typed slash command. An unknown command reports rather than doing
/// nothing. The input carries its leading slash, for example `/nope`.
pub fn run_slash_command(_input: &str) -> SlashOutcome {
    todo!("run_slash_command is unimplemented in the red stage")
}
