//! Discovery tests. The help screen is generated from the binding table, so the
//! two cannot drift. The slash list filters as the user types, and an unknown
//! command reports rather than doing nothing. See `SPEC-tui-experience` section 9.

use rho_tui::{
    SlashOutcome, bindings, filter_slash_commands, help_rows, run_slash_command, slash_commands,
};

#[test]
fn help_rows_match_the_binding_table() {
    // The help screen is generated from the binding table. Read the binding table,
    // render the help, and assert every binding appears. A hand-written help list
    // that happens to match is the defect this test exists to catch, so every key
    // and every summary must be present.
    let table = bindings();
    let rendered = help_rows().join("\n");
    assert!(!table.is_empty(), "the binding table is empty");
    for binding in table {
        assert!(
            rendered.contains(binding.keys),
            "the help screen dropped the keys {:?}",
            binding.keys
        );
        assert!(
            rendered.contains(binding.summary),
            "the help screen dropped the summary {:?}",
            binding.summary
        );
    }
}

#[test]
fn slash_list_filters_as_you_type() {
    // Typing filters the slash list. A query keeps only the commands whose name
    // starts with it. Not a spec-named test; the stage task requires this
    // assertion, and the spec names no test for it.
    let all = slash_commands();
    assert!(!all.is_empty(), "the slash-command list is empty");

    // The empty prefix `/` keeps every command.
    let unfiltered = filter_slash_commands("/");
    assert_eq!(
        unfiltered.len(),
        all.len(),
        "the `/` prefix dropped commands from the list"
    );

    // A named prefix keeps only its command, and drops the rest.
    let guided = filter_slash_commands("/guide");
    assert!(
        guided
            .iter()
            .all(|command| command.name.starts_with("/guide")),
        "the filter kept a command that does not start with the query"
    );
    assert!(
        guided.iter().any(|command| command.name == "/guide"),
        "the filter dropped `/guide` for its own prefix"
    );
    assert!(
        guided.len() < all.len(),
        "the filter kept every command for a specific query"
    );
}

#[test]
fn unknown_slash_command_reports() {
    // An unknown command reports rather than doing nothing. Not a spec-named test;
    // the stage task requires this assertion, and the spec names no test for it.
    let outcome = run_slash_command("/nope-not-a-command");
    match outcome {
        SlashOutcome::Unknown(message) => assert!(
            message.contains("nope-not-a-command"),
            "the report did not name the unknown command: {message}"
        ),
        SlashOutcome::Run(name) => {
            panic!("an unknown command resolved to a run of {name}")
        }
    }
}

#[test]
fn guide_is_a_slash_command() {
    // `/guide` runs the two-minute tour, so the empty state and the help both name a
    // real command. Not a spec-named test; the stage task lists the guide, and the
    // spec names no test for it.
    let has_guide = slash_commands()
        .iter()
        .any(|command| command.name == "/guide");
    assert!(has_guide, "the slash list has no `/guide` command");
}

#[test]
fn the_help_screen_marks_a_binding_that_is_not_wired() {
    // The help screen became reachable, and it listed four keys that answer nothing:
    // ctrl-o, ctrl-e, alt+enter, and the arrow selection outside a panel. An interface
    // that promises a key it does not answer is the defect D-a-panel-nobody-can-open
    // describes, so an unwired binding says so on its own row.
    let rendered = help_rows().join("\n");
    for binding in bindings() {
        if binding.built {
            continue;
        }
        let row = help_rows()
            .into_iter()
            .find(|row| row.contains(binding.keys))
            .expect("every binding has a row");
        assert!(
            row.contains("not built yet"),
            "the unwired binding {:?} reads as working: {row:?}",
            binding.keys
        );
    }
    // At least one binding is wired, and a wired row carries no warning.
    let wired = bindings()
        .iter()
        .find(|binding| binding.built)
        .expect("some binding is wired");
    let row = help_rows()
        .into_iter()
        .find(|row| row.contains(wired.keys))
        .expect("every binding has a row");
    assert!(
        !row.contains("not built yet"),
        "a wired binding must not carry the warning: {row:?}"
    );
    assert!(rendered.contains("enter"), "the table still lists enter");
}
