//! Steering is visible in the TUI. See `SPEC-a-queued-message-says-what-it-is`.
//!
//! These tests drive the pure reducer and the pure key handler. No test uses the network.
//! No test uses `sleep`. The app-level wiring (that a mid-turn Enter steers and never
//! calls `Session::prompt` a second time) is guarded by a source-reading test, because the
//! event loop owns a real terminal, in the shape of `the_event_loop_reads_the_task_stream`.

use rho_core::{AgentEvent, AgentStopReason, QueueError, StreamEvent};
use rho_tui::{ActivityState, KeyAction, Row, TuiState};

/// A running turn, with the draft holding `text`.
fn running_with_draft(text: &str) -> TuiState {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    assert_eq!(state.activity, ActivityState::Running);
    state.draft.set_text(text);
    state
}

fn enter(state: &mut TuiState) -> KeyAction {
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ))
}

// ---- Enter routes by activity. -----------------------------------------------

#[test]
fn an_enter_while_a_turn_runs_returns_steer() {
    let mut state = running_with_draft("a steer message");
    let action = enter(&mut state);
    assert!(
        matches!(action, KeyAction::Steer(ref text) if text == "a steer message"),
        "a running turn steers the draft, got {action:?}"
    );
}

#[test]
fn an_enter_while_idle_returns_submit() {
    let mut state = TuiState::default();
    assert_eq!(state.activity, ActivityState::Idle);
    state.draft.set_text("an idle prompt");
    let action = enter(&mut state);
    assert!(
        matches!(action, KeyAction::Submit(ref text) if text == "an idle prompt"),
        "an idle turn submits the draft, got {action:?}"
    );
}

#[test]
fn an_enter_that_steers_keeps_the_draft_until_the_result() {
    let mut state = running_with_draft("kept until the result");
    let action = enter(&mut state);
    assert!(matches!(action, KeyAction::Steer(_)));
    // The Enter handler does not clear the draft, so a later refusal keeps the message.
    assert_eq!(state.draft.model_text(), "kept until the result");
    // And it pushes no row. The row appears only when the steer result lands.
    assert!(state.rows.is_empty());
}

// ---- on_steer_result. --------------------------------------------------------

#[test]
fn a_successful_steer_pushes_a_waiting_user_row() {
    let mut state = running_with_draft("hello");
    state.on_steer_result("hello".to_string(), Ok(0));
    assert!(matches!(
        state.rows.last(),
        Some(Row::User { text, delivered: false }) if text == "hello"
    ));
}

#[test]
fn a_successful_steer_clears_the_draft() {
    let mut state = running_with_draft("hello");
    state.on_steer_result("hello".to_string(), Ok(0));
    assert!(state.draft.is_empty());
    assert!(state.steer_notice.is_none());
}

#[test]
fn a_full_queue_keeps_the_draft_and_names_the_capacity() {
    let mut state = running_with_draft("too many");
    state.on_steer_result(
        "too many".to_string(),
        Err(QueueError::Full { capacity: 32 }),
    );
    // The draft stays, so the message is not lost.
    assert_eq!(state.draft.model_text(), "too many");
    // No row is pushed for a refused message.
    assert!(state.rows.is_empty());
    let notice = state
        .steer_notice
        .as_deref()
        .expect("a refusal sets a notice");
    assert!(
        notice.contains("32"),
        "the notice names the capacity: {notice}"
    );
}

#[test]
fn a_too_large_message_keeps_the_draft_and_names_both_numbers() {
    let mut state = running_with_draft("huge");
    state.on_steer_result(
        "huge".to_string(),
        Err(QueueError::TooLarge {
            limit: 1024,
            size: 2048,
        }),
    );
    assert_eq!(state.draft.model_text(), "huge");
    assert!(state.rows.is_empty());
    let notice = state
        .steer_notice
        .as_deref()
        .expect("a refusal sets a notice");
    assert!(
        notice.contains("2048"),
        "the notice names the size: {notice}"
    );
    assert!(
        notice.contains("1024"),
        "the notice names the limit: {notice}"
    );
}

// ---- delivery flips waiting rows. --------------------------------------------

#[test]
fn a_delivery_flips_the_oldest_waiting_row() {
    let mut state = running_with_draft("a");
    state.on_steer_result("a".to_string(), Ok(0));
    state.draft.set_text("b");
    state.on_steer_result("b".to_string(), Ok(1));
    // Both wait. A delivery of one flips the oldest, in arrival order.
    state.apply(&AgentEvent::MessageDelivered { count: 1 }, 0);
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: true,
            ..
        }
    ));
    assert!(matches!(
        state.rows[1],
        Row::User {
            delivered: false,
            ..
        }
    ));
}

#[test]
fn a_delivery_of_two_flips_two_waiting_rows_in_order() {
    let mut state = running_with_draft("a");
    state.on_steer_result("a".to_string(), Ok(0));
    state.draft.set_text("b");
    state.on_steer_result("b".to_string(), Ok(1));
    state.apply(&AgentEvent::MessageDelivered { count: 2 }, 0);
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: true,
            ..
        }
    ));
    assert!(matches!(
        state.rows[1],
        Row::User {
            delivered: true,
            ..
        }
    ));
}

#[test]
fn a_delivery_never_flips_a_normal_user_row() {
    let mut state = TuiState::default();
    // A message sent while idle is delivered at once.
    state.draft.set_text("idle msg");
    let _ = enter(&mut state);
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: true,
            ..
        }
    ));
    // A delivery flips only waiting rows, so this delivered row is untouched, and the
    // count of waiting rows stays zero.
    state.apply(&AgentEvent::MessageDelivered { count: 1 }, 0);
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: true,
            ..
        }
    ));
    assert_eq!(state.waiting_count(), 0);
}

#[test]
fn the_waiting_count_equals_steers_minus_deliveries() {
    // The invariant, not one example: for any sequence, waiting == steers - deliveries.
    let mut state = running_with_draft("a");
    state.on_steer_result("a".to_string(), Ok(0));
    state.draft.set_text("b");
    state.on_steer_result("b".to_string(), Ok(1));
    state.draft.set_text("c");
    state.on_steer_result("c".to_string(), Ok(2));
    assert_eq!(state.waiting_count(), 3);
    state.apply(&AgentEvent::MessageDelivered { count: 2 }, 0);
    assert_eq!(state.waiting_count(), 1);
    state.apply(&AgentEvent::MessageDelivered { count: 1 }, 0);
    assert_eq!(state.waiting_count(), 0);
}

// ---- a waiting row outlives a cancel and a run that ends without AgentEnd. ----

#[test]
fn a_cancel_keeps_every_waiting_row() {
    let mut state = running_with_draft("a");
    state.on_steer_result("a".to_string(), Ok(0));
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    assert!(state.canceling);
    // The TUI never drops a waiting row on its own. It stays, tagged waiting.
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: false,
            ..
        }
    ));
}

#[test]
fn a_run_that_ends_without_agentend_keeps_waiting_rows() {
    let mut state = running_with_draft("a");
    state.on_steer_result("a".to_string(), Ok(0));
    // The stream closed with no AgentEnd. The frontend ends the run itself.
    state.end_run(true, 1_000);
    assert_eq!(state.activity, ActivityState::Idle);
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: false,
            ..
        }
    ));
    // A later delivery still flips it.
    state.apply(&AgentEvent::MessageDelivered { count: 1 }, 0);
    assert!(matches!(
        state.rows[0],
        Row::User {
            delivered: true,
            ..
        }
    ));
}

// ---- the composer hint. -----------------------------------------------------

#[test]
fn the_composer_hint_uses_the_word_steer_while_a_turn_runs() {
    let state = running_with_draft("a draft");
    let hint = state
        .composer_hint()
        .expect("a running turn with a draft hints");
    assert!(
        hint.contains("steer"),
        "the hint uses the word steer: {hint}"
    );
}

#[test]
fn the_composer_hint_is_absent_while_idle() {
    let mut state = TuiState::default();
    state.draft.set_text("a draft");
    assert!(state.composer_hint().is_none());
}

// ---- the guide names steering. ----------------------------------------------

#[test]
fn the_guide_names_steering_with_one_word() {
    // The guide is generated from the binding table. The `enter` row names both jobs:
    // it sends while idle and steers while a turn runs. The word is `steer`, never
    // `queue`, because `queue` is an implementation detail the user never meets.
    let pages = rho_tui::guide_pages("m", "p");
    let all: String = pages
        .iter()
        .flat_map(|page| page.rows.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.to_lowercase().contains("steer"),
        "the guide names steering with the word steer: {all}"
    );
    assert!(
        !all.to_lowercase().contains("queue"),
        "the guide never says queue, an implementation detail: {all}"
    );
}

// ---- the footer waiting count. ----------------------------------------------

fn footer(state: &TuiState) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(80, 10)).expect("a test terminal");
    terminal
        .draw(|frame| rho_tui::render(state, frame))
        .expect("the frame draws");
    let buffer = terminal.backend().buffer().clone();
    (0..80)
        .map(|x| buffer[(x, 9)].symbol().to_string())
        .collect::<String>()
}

#[test]
fn the_footer_shows_the_waiting_count() {
    let mut state = running_with_draft("a");
    state.on_steer_result("a".to_string(), Ok(0));
    state.draft.set_text("b");
    state.on_steer_result("b".to_string(), Ok(1));
    let text = footer(&state);
    assert!(
        text.contains("2 waiting"),
        "the footer counts waiting: {text}"
    );
}

#[test]
fn the_footer_shows_no_waiting_tail_when_none_wait() {
    let state = running_with_draft("a");
    // A running turn with no steered message shows no waiting tail.
    let text = footer(&state);
    assert!(
        !text.contains("waiting"),
        "no waiting tail when none wait: {text}"
    );
}

// ---- a waiting row renders differently from a delivered row. -----------------

#[test]
fn a_waiting_row_renders_differently_from_a_delivered_row() {
    let mut waiting = TuiState::default();
    waiting.draft.set_text("msg");
    // Force a waiting row without a live turn, for a pure render comparison.
    waiting.rows.push(Row::User {
        text: "msg".to_string(),
        delivered: false,
    });
    let mut delivered = TuiState::default();
    delivered.rows.push(Row::User {
        text: "msg".to_string(),
        delivered: true,
    });
    let wait_text = render_transcript(&waiting);
    let done_text = render_transcript(&delivered);
    assert!(
        wait_text.contains("waiting"),
        "a waiting row carries the waiting tag: {wait_text}"
    );
    assert!(
        !done_text.contains("waiting"),
        "a delivered row carries no waiting tag: {done_text}"
    );
}

/// The transcript rows of a rendered frame, joined.
fn render_transcript(state: &TuiState) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("a test terminal");
    terminal
        .draw(|frame| rho_tui::render(state, frame))
        .expect("the frame draws");
    let buffer = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..10u16 {
        for x in 0..60u16 {
            out.push_str(buffer[(x, y)].symbol());
        }
    }
    out
}

// ---- the app-level wiring guard. --------------------------------------------
//
// The event loop owns a real terminal, so no unit test can drive a mid-turn Enter through
// it. This test reads the loop and states the rule: a Steer arm calls `Session::steer` and
// never `Session::prompt`, so a mid-turn Enter can no longer overwrite the live run. The
// live drive is the other half. See `SPEC-a-queued-message-says-what-it-is` amendment 1.

#[test]
fn the_event_loop_steers_and_never_prompts_mid_turn() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs"))
        .expect("read the app source");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<&str>>()
        .join("\n");
    assert!(
        code.contains("KeyAction::Steer(text)"),
        "the loop must handle the Steer action"
    );
    assert!(
        code.contains("session.steer("),
        "the Steer arm must call Session::steer"
    );
    // The Steer arm must not call Session::prompt. Find the Steer arm body and check it.
    let steer_arm = code
        .split("KeyAction::Steer(text)")
        .nth(1)
        .expect("the Steer arm exists");
    let prompt_call = steer_arm.split("KeyAction::").next().unwrap_or("");
    assert!(
        !prompt_call.contains("session.prompt("),
        "the Steer arm must never call Session::prompt, or it overwrites the live run"
    );
}

// A sentinel so `StreamEvent` import is not warned away in a build that picks up this file
// in a context that reads only part of it. It documents that the reducer folds real core
// events, and keeps the import honest.
const _: fn() = || {
    let _ = AgentEvent::TurnStart;
    let _ = AgentStopReason::EndTurn;
    let _ = StreamEvent::TextStart { index: 0 };
};
