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

/// The whole binding table. The single source of truth for the help screen.
pub fn bindings() -> &'static [Binding] {
    const TABLE: &[Binding] = &[
        Binding {
            keys: "enter",
            summary: "send the draft",
        },
        Binding {
            keys: "alt+enter",
            summary: "insert a newline",
        },
        Binding {
            keys: "ctrl-c",
            summary: "cancel the turn · press twice while idle to quit",
        },
        Binding {
            keys: "ctrl-o",
            summary: "expand or collapse the newest tool row",
        },
        Binding {
            keys: "ctrl-e",
            summary: "expand everything · press again to collapse",
        },
        Binding {
            keys: "↑ ↓",
            summary: "move the selection while the draft is empty",
        },
        Binding {
            keys: "esc",
            summary: "close a panel or collapse a row",
        },
    ];
    TABLE
}

/// The help rows, generated from the binding table, so the two cannot drift. Each row
/// carries a binding's keys and its summary, so the help can never omit a binding.
pub fn help_rows() -> Vec<String> {
    bindings()
        .iter()
        .map(|binding| format!("  {:<12} {}", binding.keys, binding.summary))
        .collect()
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
    const COMMANDS: &[SlashCommand] = &[
        SlashCommand {
            name: "/model",
            summary: "pick the model for this session",
        },
        SlashCommand {
            name: "/sessions",
            summary: "list, resume, or branch a session",
        },
        SlashCommand {
            name: "/guide",
            summary: "the two minute tour",
        },
        SlashCommand {
            name: "/help",
            summary: "every key and every command",
        },
        SlashCommand {
            name: "/quit",
            summary: "leave rho",
        },
    ];
    COMMANDS
}

/// The slash commands whose name starts with the typed query, so the list filters
/// as the user types. The query carries its leading slash, for example `/g`.
pub fn filter_slash_commands(query: &str) -> Vec<&'static SlashCommand> {
    slash_commands()
        .iter()
        .filter(|command| command.name.starts_with(query))
        .collect()
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
pub fn run_slash_command(input: &str) -> SlashOutcome {
    match slash_commands()
        .iter()
        .find(|command| command.name == input)
    {
        Some(command) => SlashOutcome::Run(command.name.to_string()),
        None => SlashOutcome::Unknown(format!("unknown command: {input}")),
    }
}
