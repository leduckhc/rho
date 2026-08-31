# SPEC-a-queued-message-says-what-it-is — the TUI states what steering does

Status: draft. No code yet. This spec wires the steering surface into the TUI.
Owning crate: `rho-tui`.
Features: F-tui-steering-is-visible (this spec), and F-message-steering (the core delivery).

## 1. The problem, in the user's words

The user typed while a turn ran. The interface said nothing about the text. The footer
showed only `◈ working ·`. The user could not tell what enter would do. So the user asked
for "message queue vs message steering clearly stated".

## 2. The finding: one mechanism, not two

There is one mechanism. Its name is **steering**. The queue is how steering holds a
message. The queue is not a second feature. See `D-steering-is-one-mechanism-not-two` and
`SPEC-steering` section 1. The `QueueError` text itself says "the steering queue", which
names the queue as part of steering.

So this spec does not invent a second mechanism. It names the one mechanism honestly in
the interface, and it uses one word per meaning.

## 3. The vocabulary

The interface uses these words, and only these, for this feature.

| Meaning | Word |
| --- | --- |
| the feature and the verb | steer, steering |
| a steered message the model has not received | waiting |
| a steered message the model has received | delivered |
| the count refused door | refused |

The word "queue" never reaches the user in TUI copy. It stays in the code and in the core
error text. The composer, the footer, the transcript row, and the guide all use "steer".
A second word for one idea is the defect this spec removes.

## 4. The sides

This is a one-crate change. `rho-tui` is the only side that writes code.

`rho-core` needs nothing new. It already gives the TUI every part it needs:

- `Session::steer(message) -> Result<usize, QueueError>` returns the position.
- `AgentEvent::MessageDelivered { count }` fires at the boundary.
- `QueueError::Full { capacity }` and `QueueError::TooLarge { limit, size }` name the
  refusals. Their `Display` text names both numbers.

The contract kinds this spec touches: the data model (`Row`), the behaviour rules (Enter
routes by activity), and the UI copy.

## 5. The contract, as compilable Rust

All of this lives in `crates/rho-tui/src/state.rs`.

### 5.1 The user row carries a delivery flag

```rust
/// One rendered transcript row.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A user message. `delivered` is false while the message waits to steer the
    /// agent. It is true once the model has received the message. A message the user
    /// sends while no turn runs is delivered at once, so it starts true.
    User { text: String, delivered: bool },
    // The other variants are unchanged: Assistant, Thinking, Agent, Task, Tool,
    // Error, and Notice.
}
```

### 5.2 Enter routes by activity

```rust
/// What the app must do after a key press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    None,
    /// Submit the given prompt text to the session. The turn is idle.
    Submit(String),
    /// Steer the given text into the running turn. The Enter handler returns this
    /// instead of `Submit` while a turn runs. The app calls `Session::steer`.
    Steer(String),
    Cancel,
    Exit,
    EditDraft(String),
}
```

The Enter handler reads `self.activity`. While `ActivityState::Running`, it returns
`KeyAction::Steer(self.draft.model_text())`. It does not clear the draft, and it does not
push a row. So a refusal keeps the draft. While idle, it returns `KeyAction::Submit` as
before. `Composer::model_text` is the non-destructive reader in
`crates/rho-tui/src/paste.rs`.

### 5.3 The reducer folds the steer result

```rust
impl TuiState {
    /// Fold the result of a steer attempt into the state.
    ///
    /// On success, clear the draft and push a waiting user row. On refusal, keep the
    /// draft and set the steer notice. A refusal never drops the draft, so the user
    /// keeps the message.
    pub fn on_steer_result(
        &mut self,
        text: String,
        result: Result<usize, rho_core::QueueError>,
    ) {
        match result {
            Ok(_position) => {
                self.draft.take();
                self.push_row(Row::User { text, delivered: false }, None);
                self.steer_notice = None;
            }
            Err(error) => {
                self.steer_notice = Some(error.to_string());
            }
        }
    }

    /// Flip the oldest `count` waiting user rows to delivered, in arrival order.
    fn mark_delivered(&mut self, count: usize) {
        let mut left = count;
        for row in &mut self.rows {
            if left == 0 {
                break;
            }
            if let Row::User { delivered, .. } = row {
                if !*delivered {
                    *delivered = true;
                    left -= 1;
                }
            }
        }
    }

    /// The number of waiting user rows, for the footer.
    pub fn waiting_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, Row::User { delivered: false, .. }))
            .count()
    }
}
```

`TuiState` gains one field.

```rust
    /// The last steer refusal text, or `None`. It shows near the composer, in `warn`.
    pub steer_notice: Option<String>,
```

`TuiState` derives `Default`, so the field defaults to `None`.

The `apply` match arm changes. `MessageDelivered` now calls `mark_delivered`.
`MessageQueued` still does nothing, because the return value of `steer` already drives the
waiting row.

```rust
    AgentEvent::MessageDelivered { count } => self.mark_delivered(*count),
    AgentEvent::MessageQueued { .. } => {}
```

## 6. What the interface says, and when

### 6.1 Before enter, while a turn runs

The composer hint reads: `Press enter to steer this message to the agent.` The hint uses
the word "steer". It replaces the idle hint while the turn runs and the draft is not empty.

### 6.2 After enter, on success

The message appears at once as a user row. The row draws a `waiting` tag in the `warn`
role. So the reader sees the text and sees that it waits.

### 6.3 At the boundary

The driver drains the queue and fires `MessageDelivered { count }`. `mark_delivered` flips
the oldest `count` waiting rows to delivered. Each row drops its `waiting` tag. If two
messages wait and `count` is two, both rows flip, in arrival order.

### 6.4 The footer

While a turn runs and messages wait, the footer reads `◈ working · {dur} · {n} waiting`.
The count is `waiting_count`. When no message waits, the footer drops the tail and reads
`◈ working · {dur}`. The change is in `footer_line` in `crates/rho-tui/src/render.rs`.

## 7. Every refusal

A steered message can be refused. The refusal must never look like an acceptance.

- `QueueError::Full { capacity }`: the message is not pushed. `on_steer_result` keeps the
  draft. It sets `steer_notice` to the error text, which names the capacity. The composer
  still holds the message.
- `QueueError::TooLarge { limit, size }`: the same. The error text names the size and the
  limit. The composer still holds the message.

The refused text stays in the composer. It never vanishes. No row is pushed for a refused
message.

## 8. Cancellation and failure

- On cancel: `SPEC-steering` section 6 says the core keeps the queue. The TUI keeps every
  waiting row. A waiting row stays on screen with its `waiting` tag. The core delivers it
  on the next run.
- On a run that ends without `AgentEnd`: a live review found this path. The core keeps the
  message. The TUI keeps the waiting row. So the message is visible, and the next run
  delivers it and flips the row. The TUI never drops a waiting row on its own.

A `clear` of the transcript is explicit, never a side effect.

## 9. The extension point

A frontend that is not the TUI steers through the same `Session::steer`. See
F-steer-command over ACP. The `delivered` flag is a rendering choice local to `rho-tui`.
A new frontend reads the same `MessageDelivered` event and chooses its own view.

## 10. Test cases

All tests drive the pure reducer and the pure key handler. No test uses the network. No
test uses `sleep`.

- `an_enter_while_a_turn_runs_returns_steer` — with `activity` running, the Enter handler
  returns `KeyAction::Steer` with the model text.
- `an_enter_while_idle_returns_submit` — with `activity` idle, the Enter handler returns
  `KeyAction::Submit`, unchanged.
- `an_enter_that_steers_keeps_the_draft_until_the_result` — the Enter handler does not
  clear the draft, so a later refusal keeps the message.
- `a_successful_steer_pushes_a_waiting_user_row` — `on_steer_result` with `Ok` pushes
  `Row::User { delivered: false }`.
- `a_successful_steer_clears_the_draft` — `on_steer_result` with `Ok` empties the composer.
- `a_full_queue_keeps_the_draft_and_names_the_capacity` — `on_steer_result` with
  `Err(Full { capacity })` keeps the draft, pushes no row, and sets `steer_notice` to text
  that names the capacity.
- `a_too_large_message_keeps_the_draft_and_names_both_numbers` — `on_steer_result` with
  `Err(TooLarge { limit, size })` keeps the draft, pushes no row, and sets `steer_notice`
  to text that names the size and the limit.
- `a_delivery_flips_the_oldest_waiting_row` — `MessageDelivered { count: 1 }` flips the
  oldest waiting row and leaves a later one waiting.
- `a_delivery_of_two_flips_two_waiting_rows_in_order` — `MessageDelivered { count: 2 }`
  flips the two oldest waiting rows.
- `a_delivery_never_flips_a_normal_user_row` — a row sent while idle stays `delivered:
  true` and is never counted by `mark_delivered`.
- `the_waiting_count_equals_steers_minus_deliveries` — for any sequence of steers and
  deliveries, `waiting_count` equals the steers accepted minus the messages delivered. The
  invariant, not one example.
- `the_footer_shows_the_waiting_count` — with two waiting rows, the footer text holds
  `2 waiting`.
- `the_footer_shows_no_waiting_tail_when_none_wait` — with no waiting row, the footer text
  holds no `waiting`.
- `a_waiting_row_renders_differently_from_a_delivered_row` — the render marks a
  `delivered: false` user row with the `waiting` tag and a `delivered: true` row without.
- `a_cancel_keeps_every_waiting_row` — a cancel leaves each `delivered: false` row in
  place.
- `a_run_that_ends_without_agentend_keeps_waiting_rows` — with no `AgentEnd`, each waiting
  row stays, and a later `MessageDelivered` flips it.
- `the_composer_hint_uses_the_word_steer_while_a_turn_runs` — the hint text holds `steer`
  while the turn runs and the draft is not empty.
- `the_guide_names_steering_with_one_word` — the guide steering entry holds `steer` and
  never holds `queue`.

## 11. Out of scope

- The core queue, its bounds, and its byte cap. `SPEC-steering` owns those.
- A change to `Session::steer` or to any `rho-core` type.
- Delivering a message inside a provider request. The boundary is the delivery point.
- A priority order or a de-duplication among waiting messages.
- Editing a message the model already received. The prompt prefix is append-only.
- The ACP frontend copy. F-steer-command owns that surface.
- Removing a waiting row from the composer's own history recall.
