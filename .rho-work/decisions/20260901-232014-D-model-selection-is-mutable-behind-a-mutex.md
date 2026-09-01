# D-model-selection-is-mutable-behind-a-mutex

Date: 20260901-232014

## The question

`/model <id>` and `/effort <level>` must take effect mid-session. `SessionConfig` is copied
into `SessionInner` and then hidden behind an `Arc`, and `Session::with_queue` rebuilds the
whole `SessionInner` by `Arc::try_unwrap`, which fails once the driver spawns a task and
clones the `Arc`. So the frontend cannot swap the whole config.

Where does the mutable state live, and what may change?

## The decision

`SessionInner` gains one interior-mutable field, `selection: Mutex<ModelSelection>`. The
field carries the model id and the reasoning effort, and nothing else.

- `Session::selection(&self) -> ModelSelection` reads the current pair.
- `Session::set_selection(&self, ModelSelection)` writes it. The write is synchronous,
  needs `&self`, and holds the lock only long enough to swap.
- `Driver::build_request` reads the pair from the mutex at the start of every turn. The
  running turn is unaffected. The next turn uses the new pair.
- `SessionConfig::model` and `SessionConfig::reasoning_effort` remain the **initial**
  values. `Session::with_config` primes the mutex from them, so no caller changes.
- **Provider stays fixed for the life of the session.** A new provider needs a new
  `Arc<dyn Provider>`, a new credential resolution, a new tool-set intersection, and a new
  system prompt. All four cross the session boundary. Cross-provider switching is not built
  in v1.

## What this rules out

- A `Session::with_selection` that returns `Self` and needs `Arc::try_unwrap`. It would
  panic once the driver holds a task handle, which is exactly when the user reaches for
  `/model`.
- Mutating the whole `SessionConfig` mid-session. A caller could then change the session
  root, the approval policy, or the sandbox mode, and a security review already refused
  that surface once.
- A `RwLock` around the whole config. Read-heavy, write-rare says `Mutex`, and the write
  path already runs off the hot path.
- A provider switch through `/model`. It stays a session boundary.
- Reading the model from `SessionConfig` at request-build time. Two sources of truth would
  drift, so `build_request` reads only from the mutex.

## Why

The smallest interior-mutable slice keeps every other invariant. The mutex holds two
values whose only readers are `Session::selection` and `Driver::build_request`, so the lock
is uncontended and cannot deadlock. `AGENTS.md` says a contract is closed for modification
and open for extension: a new field, when needed, is a new mutex or a new struct field,
not an edit to `SessionConfig`.
