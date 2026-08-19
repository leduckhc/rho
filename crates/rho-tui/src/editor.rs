//! The external editor contract.
//!
//! The composer opens the draft in an editor. The editor name comes from the
//! environment. The command splits on whitespace, and it never reaches a shell.
//! See `SPEC-tui-inline-and-composer` section 6.6.

/// The editor command, from `$VISUAL`, then `$EDITOR`, then `vi`.
///
/// The caller passes the values, so no test reads the real environment. An empty
/// value counts as unset.
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> String {
    for text in [visual, editor].into_iter().flatten() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "vi".to_string()
}

/// The program and its arguments, split from an editor value on whitespace.
///
/// The first token is the program. The rest are arguments. The loop appends the
/// temporary file path last. It never passes the value to a shell, and it never uses
/// `sh -c`. So `vi; rm -rf ~` splits to inert tokens, and the second command never runs.
pub fn editor_argv(command: &str) -> Vec<String> {
    command.split_whitespace().map(str::to_string).collect()
}
