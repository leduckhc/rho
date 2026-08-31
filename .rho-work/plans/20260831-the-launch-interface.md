# Plan: the launch interface, nine features

Owner: controller. Written 20260831. Specs: the six files stamped `20260831-2015*` under
`docs/specs/`. Decisions: the sixteen files stamped `20260831-2015*` under
`.rho-work/decisions/`.

This plan holds the order of work, who owns which file, and the gates. It does not repeat a
contract. Each contract lives in its spec.

## What the nine features became

| # | user's words | spec | size |
| --- | --- | --- | --- |
| 1 | animation and a live duration counter | `SPEC-the-turn-clock-and-the-working-state` | M |
| 2 | queue versus steering, clearly stated | `SPEC-a-queued-message-says-what-it-is` | M, and it fixes a defect |
| 3 | thinking tokens and duration | `SPEC-usage-carries-reasoning` | M, blocked in part |
| 4 | model selection and run config | `SPEC-choose-a-model-and-configure-a-run` | L |
| 5, 6, 7 | three tool levels, edit diff, other rows | `SPEC-the-tool-row-has-three-levels` | L |
| 8, 9 | selection with scrolling, jump to bottom | `SPEC-select-text-and-find-the-bottom` | L |

## Two defects the spec work uncovered

Both were measured live, not read.

1. **A prompt typed during a turn is lost.** `KeyAction::Submit` has no guard on activity, so
   `crates/rho-tui/src/app.rs` calls `Session::prompt` again and overwrites `*events` and
   `*cancel`. A live drive submitted `ALPHA`, then `BETA` mid-turn. Only `ALPHA` was
   answered. `BETA` drew a row and vanished, with no answer and no error. `Session::steer`
   exists, is tested, and no frontend calls it. So feature 2 is a correctness fix first and a
   wording fix second.
2. **The working state never animates.** `state.tick` is incremented nowhere, and the loop
   has no timer. `--no-motion` renders identically to the default. Feature 1 makes two false
   claims in `docs/guide/status.md` true.

## Phases

A phase is a barrier only where a contract forces one. Inside a phase, tasks own separate
files.

### Phase 0: review the contracts

No code starts until this closes. AGENTS.md step 3 requires a contract review before either
side implements. The six open questions below belong to this phase.

### Phase 1: the core contracts

Two tasks, no shared file, both in `rho-core` and the providers. They must land before the
interface work that reads them.

- **T1 `Usage` carries reasoning.** `rho-core/src/usage.rs`, then each provider crate, then
  `rho-provider-testkit`.
- **T2 the model catalogue.** `rho-core` gains `ModelCatalog` and
  `Provider::catalog`. Then bedrock and openrouter implement it, and azure returns `None`.

### Phase 2: the interface, partitioned by function

`crates/rho-tui/src/render.rs` is 1895 lines and four tasks touch it. They are assigned by
function, so no two edit one hunk.

- **T3 the turn clock.** `app.rs` gains the timer arm. `state.rs` gains `on_tick`.
  `render.rs`: `footer_line`, `footer_hints`, `apply_sweep`.
- **T4 steering is visible.** `app.rs` Enter routing, `state.rs` row and reducer,
  `render.rs`: `composer_lines` and the footer count. **T3 and T4 both touch the footer, so
  T4 waits for T3.**
- **T5 three tool levels and the edit diff.** `concise.rs`, `state.rs` `on_tool_end` and the
  body store, `render.rs`: `tool_header` and `tool_body`. `rho-tools/src/edit.rs` for the
  diff payload. Needs T1 for nothing, so it runs beside T3.
- **T6 selection and the jump affordance.** A new module, `scroll.rs`, `state.rs` selection
  fields, `render.rs`: `transcript_window` and `draw_rail`. Runs beside T5.
- **T7 the picker and the run commands.** Needs T2. `render.rs`: `panel_lines` and a new
  panel. `bindings.rs` and the slash list.

### Phase 3: prove it

- The full gate, run the way CI runs it.
- A mutation proof per new test. Break the line, watch the named test fail, restore from a
  copy, never with `git checkout`.
- A live drive against Bedrock, in a pty, for every interface change. The pty driver from
  the last verification pass is the harness.
- `bench/check-dead-surface.py` must not grow. This work closes seven of its entries, so the
  count must fall.

### Phase 4: the record

- Amend each spec where the code diverged.
- Update `docs/features.md`, `docs/guide/status.md`, and `docs/guide/terminal.md`.
- Write `docs/verification/the-launch-interface.md` with the real commands and output.

## Six questions the review must answer

Each one blocks a task. None is a matter of taste.

1. **The repaint cost.** The clock spec bounds the idle cost and not the running cost. Ten
   redraws a second against a measured 6.5 ms frame needs a number, per the rule that a
   performance claim carries its measurement. Blocks T3.
2. **Reasoning tokens on two providers rho cannot test.** Only Bedrock credentials exist
   here, and Bedrock reports no reasoning count. The OpenRouter and Azure field names are
   unverified. rho must not claim a number it cannot prove, so decide: ship the contract with
   Bedrock's `None` and leave the other two unimplemented and unclaimed, or hold T1.
3. **A contract test nobody runs.** `rho-provider-testkit::run_all` has no caller. Adding a
   check to it is theatre until a caller exists. Decide whether T1 wires the caller.
4. **The tool body is discarded today.** `state.rs` `on_tool_end` drops `output.content`, so
   no diff can reach a row. T5 must change the row body store, which is a data-model change
   inside `rho-tui`. Confirm the shape before code.
5. **Selection versus freezing.** `F-freeze-upward` moves a final row into the terminal's own
   scrollback so that a native drag-select works. An app-owned selection wants the opposite.
   One of the two must give way, and an owner must say which. Blocks T6.
6. **`fit_to_width` splits a grapheme cluster.** A review already found it. Decide whether T5
   fixes it or a separate bug-fix lane does, and pin it with a test either way.

## What this plan refuses

- No task starts before its contract review closes.
- No feature ships on a green suite alone. Each one is driven live in a pty.
- No claim reaches `docs/guide/status.md` before a drive proves it. Two claims there are
  false today, and that is what feature 1 exists to correct.
- No new dependency arrives by hand. `base64` and `unicode-segmentation` come in with
  `cargo add`.
