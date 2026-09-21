# D-the-provider-is-mutable-behind-a-mutex

Date: 20260918-204100

## The question

A user picks a model from a foreign provider in the `/model` picker. The session must run
that model on that provider, without a restart. The provider is `provider: Arc<dyn
Provider>` on the private `SessionInner`, at `crates/rho-core/src/agent.rs` line 330. The
session shares that `Arc` the moment the driver spawns a task. So the frontend cannot swap
the whole `SessionInner`. Where does the mutable provider live, and when does a swap take
effect?

## The decision

The provider, its config name, the model, and the effort become one interior-mutable unit.
`SessionInner` drops the separate `provider` and `selection` fields and gains one field:

```rust
struct RunningState {
    provider: Arc<dyn Provider>,
    /// The config provider name, such as `openrouter` or `xdent-claude`. Not the wire
    /// `Provider::id`, which is only the protocol. See the id-versus-name section below.
    provider_name: String,
    model: String,
    reasoning_effort: Option<crate::ReasoningEffort>,
}
// SessionInner:
//   running: std::sync::Mutex<RunningState>,
```

- `run_turn` reads the whole unit once, under one lock, at the start of a turn. It clones
  the `Arc`, the model, and the effort out, then drops the lock. See `agent.rs` line 737.
- `build_request` no longer reads a mutex. `run_turn` passes it the model and the effort it
  already read. So the request and the provider that streams it come from one snapshot.
- `Session::apply_selection` swaps the unit under the same lock. It replaces the provider,
  the name, the model, and the effort together, in one lock hold.
- A running turn keeps its own clones. So a swap never changes the provider or the model
  under a streaming request.
- The next turn reads the unit again, so it uses the new provider and the new model.

This follows `D-model-selection-is-mutable-behind-a-mutex`. That decision put the model and
the effort behind a mutex and read them once per turn. This decision widens that mutex to
hold the provider and its name too, so the one snapshot covers all four.

## Why one unit, and not two mutexes

An earlier draft kept the provider in one mutex and the model in another. A review found a
race. `build_request` read the model from the selection mutex at the top of `run_turn`.
`run_turn` read the provider from a separate mutex after `build_request` returned. A
concurrent `apply_selection` could write the new provider and the new model between those
two reads. The turn then sent the old model id to the new provider, which rejects it.

Two mutexes cannot be read as one atomic snapshot. So the provider and the model must live
in one mutex, read in one lock hold. One unit makes the invariant true, not merely claimed.

## Why the config name lives in the unit, and not `Provider::id`

`Provider::id` returns the wire protocol, not the config name. Anthropic returns
`anthropic`, and a named `[[providers]]` entry `xdent-claude` that speaks the anthropic
protocol also returns `anthropic`. So `id` cannot name the running provider.

Four readers need the config name: `Session::selection` for the header, the switch guard
that decides whether a pick is a switch, `record_model_change` for the session file, and
the picker's `(current)` tag. All four read `provider_name` from the unit. So a named
entry reports and records its own id, and re-picking the same named entry is not a switch.

`with_config` seeds `provider_name` from `Provider::id` as a safe default, because a
built-in's id equals its name. `Session::with_provider_name` overrides it with the resolved
config name, and `rho-cli` calls it. The override rebuilds `SessionInner` by
`Arc::try_unwrap`, exactly as `with_queue` does, so it must run before the session is
shared.

## Why a `Mutex` and not `Arc::try_unwrap`

`Arc::try_unwrap` returns the inner value only when the `Arc` has one strong reference. The
driver clones the session `Arc` as soon as it spawns a turn task. So `try_unwrap` fails, or
panics on an `expect`, at exactly the moment a user reaches for `/model`. A `Mutex` swaps a
shared value in place, so it never needs sole ownership.

## Why `std::sync::Mutex` and not `RwLock` or the async mutex

The write is rare, and the read is one clone per turn, off the hot path. A plain `Mutex` is
the smallest tool that fits. The lock is held only to clone or to swap, never across an
`await`. So it cannot deadlock and it cannot stall the async runtime.

## What this rules out

- **Two mutexes for the provider and the model.** They cannot be read atomically, so the
  turn could pair a new provider with an old model.
- **`Provider::id` as the running-provider identity.** It is the protocol, so a named entry
  would report and record the wrong name and re-pick would rebuild.
- **`Arc::try_unwrap` to rebuild `SessionInner` on a swap.** It panics once the session is
  shared, which is the normal state during a run.
- **A whole-`SessionConfig` swap.** A caller could then change the session root, the
  approval policy, or the sandbox mode. A security review refused that surface once.
- **A swap that mutates a turn in flight.** The turn holds its own clones, so its prompt
  prefix and its provider stay fixed for the life of that turn.

## Why

One unit read once per turn makes the whole selection atomic. The model, the effort, the
provider, and its name always agree on the wire, because one snapshot carries them all.
