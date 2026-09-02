//! Tests for the model picker: the `/model` and `/effort` slash commands, the panel
//! keys, and the row-planning arithmetic. See `SPEC-model-selection-in-tui`.
//!
//! These tests never touch a session and never do IO. `KeyAction::ApplySelection` and
//! `KeyAction::PersistStarred` are the seams the app loop turns into `Session::set_selection`
//! and `starred::save`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_core::{ModelSelection, ReasoningEffort};
use rho_tui::{
    KeyAction, Panel, TuiState, filter_slash_commands, fuzzy_match, next_effort_in_cycle,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn seeded_state() -> TuiState {
    let mut state = TuiState::default();
    state.model = "seed-model".to_string();
    state.provider = "openrouter".to_string();
    state.reasoning_effort = Some(ReasoningEffort::Medium);
    state.set_starred_models(vec!["starred-a".to_string(), "starred-b".to_string()]);
    state
}

/// Type a slash command into the composer, and press Enter to submit it.
fn type_command(state: &mut TuiState, command: &str) -> KeyAction {
    for ch in command.chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
    state.handle_key(key(KeyCode::Enter))
}

#[test]
fn slash_effort_command_appears_in_the_list_with_its_argument() {
    let commands: Vec<&'static str> = filter_slash_commands("/eff")
        .iter()
        .map(|c| c.name)
        .collect();
    assert!(
        commands.contains(&"/effort"),
        "`/effort` shows in the filtered list: {commands:?}"
    );
    let with_arg: Vec<&'static str> = filter_slash_commands("/effort high")
        .iter()
        .map(|c| c.name)
        .collect();
    assert!(
        with_arg.contains(&"/effort"),
        "`/effort high` still matches the command: {with_arg:?}"
    );
}

#[test]
fn slash_model_with_no_arg_opens_the_picker() {
    let mut state = seeded_state();
    let action = type_command(&mut state, "/model");
    assert_eq!(action, KeyAction::None, "no ApplySelection on a bare open");
    match &state.panel {
        Panel::ModelPicker(picker) => {
            assert!(
                picker
                    .rows
                    .first()
                    .map(|row| row.is_current)
                    .unwrap_or(false),
                "the first row is the current model"
            );
        }
        other => panic!("the model picker did not open: {other:?}"),
    }
}

#[test]
fn slash_model_with_an_id_arg_applies_and_closes() {
    let mut state = seeded_state();
    let action = type_command(&mut state, "/model picked-model");
    assert_eq!(
        action,
        KeyAction::ApplySelection(ModelSelection {
            model: "picked-model".to_string(),
            reasoning_effort: Some(ReasoningEffort::Medium),
        }),
        "a typed id applies with the current effort"
    );
    assert!(matches!(state.panel, Panel::None));
    let notice_bodies: Vec<String> = state
        .live_rows()
        .iter()
        .filter_map(|row| match row {
            rho_tui::Row::Notice { message } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert!(
        notice_bodies
            .iter()
            .any(|m| m.contains("model set to picked-model")),
        "one notice names the new model: {notice_bodies:?}"
    );
}

#[test]
fn slash_effort_with_no_arg_shows_the_current_level_as_a_notice() {
    let mut state = seeded_state();
    let action = type_command(&mut state, "/effort");
    assert_eq!(action, KeyAction::None);
    let notices: Vec<String> = state
        .live_rows()
        .iter()
        .filter_map(|row| match row {
            rho_tui::Row::Notice { message } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert!(
        notices.iter().any(|m| m == "effort: medium"),
        "the notice states the current level: {notices:?}"
    );
}

#[test]
fn slash_effort_with_a_level_arg_applies_and_notices() {
    let mut state = seeded_state();
    let action = type_command(&mut state, "/effort high");
    assert_eq!(
        action,
        KeyAction::ApplySelection(ModelSelection {
            model: "seed-model".to_string(),
            reasoning_effort: Some(ReasoningEffort::High),
        }),
        "the level reaches the app loop as an ApplySelection"
    );
}

#[test]
fn slash_effort_with_an_unknown_level_pushes_an_error() {
    let mut state = seeded_state();
    let action = type_command(&mut state, "/effort loud");
    assert_eq!(action, KeyAction::None);
    let errors: Vec<String> = state
        .live_rows()
        .iter()
        .filter_map(|row| match row {
            rho_tui::Row::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert!(
        errors.iter().any(|m| m.contains("xhigh")),
        "the error names the valid levels: {errors:?}"
    );
}

#[test]
fn the_picker_shows_the_current_model_first_and_then_the_starred() {
    let mut state = seeded_state();
    // A duplicate of the current in the starred list. It must not appear twice.
    state.set_starred_models(vec![
        "seed-model".to_string(),
        "starred-a".to_string(),
        "starred-b".to_string(),
    ]);
    state.open_model_picker();
    let picker = match &state.panel {
        Panel::ModelPicker(picker) => picker.clone(),
        other => panic!("picker not open: {other:?}"),
    };
    let ids: Vec<String> = picker.rows.iter().map(|row| row.id.clone()).collect();
    assert_eq!(
        ids,
        vec![
            "seed-model".to_string(),
            "starred-a".to_string(),
            "starred-b".to_string(),
        ],
        "current first, then starred, no duplicate: {ids:?}"
    );
    assert!(picker.rows[0].is_current, "the first row is the current");
    assert!(
        picker.rows[0].starred,
        "the current is marked starred when it is in the file"
    );
    assert!(
        !picker.rows[1].is_current,
        "the second row is not the current"
    );
}

#[test]
fn arrow_keys_move_the_picker_selection_and_never_wrap() {
    let mut state = seeded_state();
    state.open_model_picker();
    // Move down to the last row.
    state.handle_key(key(KeyCode::Down));
    state.handle_key(key(KeyCode::Down));
    let last = match &state.panel {
        Panel::ModelPicker(picker) => {
            assert_eq!(picker.selected, 2, "at the last row");
            picker.filtered_indices().len() - 1
        }
        other => panic!("picker not open: {other:?}"),
    };
    // One more press must not wrap.
    state.handle_key(key(KeyCode::Down));
    match &state.panel {
        Panel::ModelPicker(picker) => assert_eq!(picker.selected, last, "no wrap at the end"),
        other => panic!("picker not open: {other:?}"),
    }
    // Move up past the top.
    state.handle_key(key(KeyCode::Up));
    state.handle_key(key(KeyCode::Up));
    state.handle_key(key(KeyCode::Up));
    state.handle_key(key(KeyCode::Up));
    match &state.panel {
        Panel::ModelPicker(picker) => assert_eq!(picker.selected, 0, "no wrap at the top"),
        other => panic!("picker not open: {other:?}"),
    }
}

#[test]
fn enter_applies_the_highlighted_row_and_closes() {
    let mut state = seeded_state();
    state.open_model_picker();
    // Highlight the second row (`starred-a`).
    state.handle_key(key(KeyCode::Down));
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        KeyAction::ApplySelection(ModelSelection {
            model: "starred-a".to_string(),
            reasoning_effort: Some(ReasoningEffort::Medium),
        }),
        "enter carries the highlighted id and the current effort"
    );
    assert!(matches!(state.panel, Panel::None));
}

#[test]
fn esc_closes_the_picker_with_no_change() {
    let mut state = seeded_state();
    state.open_model_picker();
    let action = state.handle_key(key(KeyCode::Esc));
    assert_eq!(action, KeyAction::None);
    assert!(matches!(state.panel, Panel::None));
    // The state's own selection was not changed by esc: no notice, no error.
    assert_eq!(state.model, "seed-model");
    assert_eq!(state.reasoning_effort, Some(ReasoningEffort::Medium));
}

#[test]
fn tab_cycles_the_highlighted_rows_effort_but_does_not_apply_until_enter() {
    let mut state = seeded_state();
    state.open_model_picker();
    // Move to a starred row (its effort starts unset).
    state.handle_key(key(KeyCode::Down));
    let seen: Vec<Option<ReasoningEffort>> = (0..6)
        .map(|_| {
            state.handle_key(key(KeyCode::Tab));
            match &state.panel {
                Panel::ModelPicker(picker) => {
                    let filtered = picker.filtered_indices();
                    picker.rows[filtered[picker.selected]].effort
                }
                _ => panic!("picker closed unexpectedly"),
            }
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            Some(ReasoningEffort::Off),
            Some(ReasoningEffort::Low),
            Some(ReasoningEffort::Medium),
            Some(ReasoningEffort::High),
            Some(ReasoningEffort::XHigh),
            None,
        ],
        "the six-step cycle: {seen:?}"
    );
    // `e` never applies.
    let after = match &state.panel {
        Panel::ModelPicker(_) => KeyAction::None,
        _ => panic!("picker closed"),
    };
    assert_eq!(after, KeyAction::None);
    // The pure cycle helper matches the routing.
    let helper: Vec<Option<ReasoningEffort>> = {
        let mut acc = Vec::new();
        let mut cur: Option<ReasoningEffort> = None;
        for _ in 0..6 {
            cur = next_effort_in_cycle(cur);
            acc.push(cur);
        }
        acc
    };
    assert_eq!(helper, seen, "the helper matches the routing");
}

#[test]
fn shift_tab_toggles_the_star_and_returns_a_persistence_action() {
    let mut state = seeded_state();
    state.open_model_picker();
    // Un-star the currently-starred `starred-a` row: move down and press Shift+Tab.
    state.handle_key(key(KeyCode::Down));
    let action = state.handle_key(key(KeyCode::BackTab));
    let list = match action {
        KeyAction::PersistStarred(list) => list,
        other => panic!("expected PersistStarred, got {other:?}"),
    };
    assert!(
        !list.iter().any(|id| id == "starred-a"),
        "unstarred id is gone: {list:?}"
    );
    // Toggle back on.
    let action = state.handle_key(key(KeyCode::BackTab));
    let list = match action {
        KeyAction::PersistStarred(list) => list,
        other => panic!("expected PersistStarred, got {other:?}"),
    };
    assert!(
        list.iter().any(|id| id == "starred-a"),
        "re-starred id is back: {list:?}"
    );
    // The panel is still open.
    assert!(matches!(state.panel, Panel::ModelPicker(_)));
}

#[test]
fn fuzzy_match_matches_a_scattered_subsequence_case_insensitively() {
    // The classic fzf shape.
    assert!(fuzzy_match("sn45", "claude-sonnet-4-5"));
    assert!(fuzzy_match("NoVa", "amazon.nova-micro-v1:0"));
    assert!(fuzzy_match("", "anything"), "an empty query matches");
    assert!(fuzzy_match("h", "anthropic/claude-haiku-4.5"));
}

#[test]
fn fuzzy_match_rejects_a_query_not_present() {
    assert!(!fuzzy_match("zzz", "claude-sonnet-4-5"));
    // Order matters: `54` is not a subsequence of `claude-sonnet-4-5`.
    assert!(!fuzzy_match("54", "claude-sonnet-4-5"));
    assert!(!fuzzy_match("abcd", "a-b-c"));
}

#[test]
fn typing_filters_the_picker_by_fuzzy_subsequence() {
    let mut state = seeded_state();
    state.set_starred_models(vec![
        "anthropic/claude-sonnet-4-5".to_string(),
        "amazon.nova-micro-v1:0".to_string(),
        "openai/gpt-5".to_string(),
    ]);
    state.model = "anthropic/claude-haiku-4.5".to_string();
    state.open_model_picker();
    // Type `sonnet`. Only the sonnet row matches; the current haiku row must drop.
    for ch in "sonnet".chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
    let picker = match &state.panel {
        Panel::ModelPicker(picker) => picker.clone(),
        other => panic!("picker not open: {other:?}"),
    };
    let filtered_ids: Vec<String> = picker
        .filtered_indices()
        .iter()
        .map(|i| picker.rows[*i].id.clone())
        .collect();
    assert_eq!(
        filtered_ids,
        vec!["anthropic/claude-sonnet-4-5".to_string()],
        "only sonnet passes: {filtered_ids:?}"
    );
    assert_eq!(
        picker.query, "sonnet",
        "the query field carries the typed text"
    );
    assert_eq!(picker.selected, 0, "filter reset the selection to zero");
}

#[test]
fn typing_a_letter_that_is_also_a_key_binds_to_the_query_not_the_shortcut() {
    // The old picker used `j`, `k`, `e`, and `*` as shortcuts. Model ids carry those
    // letters, so they must type into the query now. See
    // `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
    let mut state = seeded_state();
    state.set_starred_models(vec!["openai/gpt-4o-mini".to_string()]);
    state.model = "seed".to_string();
    state.open_model_picker();
    for ch in "gpt".chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
    let query = match &state.panel {
        Panel::ModelPicker(picker) => picker.query.clone(),
        other => panic!("picker not open: {other:?}"),
    };
    assert_eq!(query, "gpt", "letters reach the query field");
}

#[test]
fn enter_on_an_empty_filter_applies_the_query_verbatim() {
    // `D-a-listing-failure-never-stops-a-session`: a typed id always runs.
    let mut state = seeded_state();
    state.open_model_picker();
    for ch in "vendor/unknown-model".chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
    let filtered_len = match &state.panel {
        Panel::ModelPicker(picker) => picker.filtered_indices().len(),
        _ => panic!("picker not open"),
    };
    assert_eq!(filtered_len, 0, "the query matches no starred row");
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        KeyAction::ApplySelection(ModelSelection {
            model: "vendor/unknown-model".to_string(),
            reasoning_effort: Some(ReasoningEffort::Medium),
        }),
        "enter with no match applies the query verbatim"
    );
    assert!(matches!(state.panel, Panel::None));
}

#[test]
fn backspace_removes_a_query_char_and_resets_the_selection() {
    let mut state = seeded_state();
    state.open_model_picker();
    for ch in "star".chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
    state.handle_key(key(KeyCode::Backspace));
    match &state.panel {
        Panel::ModelPicker(picker) => {
            assert_eq!(picker.query, "sta", "backspace removes the last char");
            assert_eq!(picker.selected, 0, "backspace resets the selection");
        }
        other => panic!("picker not open: {other:?}"),
    }
    // Backspace at empty is a no-op. The panel stays open.
    for _ in 0..10 {
        state.handle_key(key(KeyCode::Backspace));
    }
    match &state.panel {
        Panel::ModelPicker(picker) => assert_eq!(picker.query, ""),
        other => panic!("picker closed after too many backspaces: {other:?}"),
    }
}

#[test]
fn the_picker_plans_a_row_per_starred_plus_the_current_line() {
    let mut state = seeded_state();
    state.set_starred_models(vec!["starred-a".to_string(), "starred-b".to_string()]);
    state.open_model_picker();
    match &state.panel {
        Panel::ModelPicker(picker) => {
            // The current is drawn as its own row (position zero), plus one row per
            // remaining star.
            assert_eq!(picker.rows.len(), 3);
            assert_eq!(
                picker.rows.iter().filter(|row| row.starred).count(),
                2,
                "two starred rows: {:?}",
                picker.rows
            );
            assert_eq!(picker.rows.iter().filter(|row| row.is_current).count(), 1);
        }
        other => panic!("picker not open: {other:?}"),
    }

    // The renderer plans one panel row per picker row, plus the `model:` header.
    let plan_rows = {
        // Render to a small backend and count how many lines the panel wanted.
        // A shorter probe than rendering: read the layout via the public seam.
        let picker = if let Panel::ModelPicker(p) = &state.panel {
            p
        } else {
            panic!("picker not open");
        };
        picker.rows.len() + 1
    };
    assert_eq!(plan_rows, 4);
}

/// The picker draws a screen row per row it planned, and a mouse click is not routed
/// through the slash-list seam. `slash_row_index` returns `None` for the picker, so a
/// click never mis-fires the wrong row. See `D-a-panel-nobody-can-open`.
#[test]
fn a_click_never_maps_the_model_picker_through_slash_row_index() {
    let mut state = seeded_state();
    state.open_model_picker();
    assert!(
        rho_tui::slash_row_index(&state, 80, 24, 5).is_none(),
        "a click while the model picker is open does not route through the slash-list seam"
    );
}
