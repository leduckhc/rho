# D-session-carries-an-optional-recorder — a mid-session model change writes a record

**Decision:** `Session` carries an optional `SessionRecorder`. When a recorder is
attached, `Session::set_selection` writes a `ModelChange` record. The provider id
comes from the session's provider, because the provider is fixed for the life of the
session.

**Reason:** a mid-session model switch must be visible in the session file, so a
reader knows which model answered which turn. The recorder is optional because not
every session writes a file, and the core must stay free of filesystem assumptions.

**Rule:** a session built without a recorder writes no record. `Session::with_recorder`
attaches one. The interactive TUI path still records nothing today; wiring it is a
separate feature.

---

**Note added 20260918.** The feature described above is now built, and its mechanism changed.

`Session::with_recorder` and the `recorder` field on `Session` are deleted. The
`record_model_change` branch of `Session::set_selection` is also deleted. The core no longer
knows about recording.

The replacement: `App` in `rho-tui` holds `Option<SessionRecorder>` directly. Every record
folds through `record_turn` in `crates/rho-tui/src/app.rs`. `rho-cli::run_interactive` opens
the `Recording`, calls `App::with_recorder`, and keeps the `Recording` alive for the whole
run. The `drop(recording)` after `app.run()` is deliberate: it holds the session lock.

The ruling that "a session built without a recorder writes no record" still holds. Only the
attachment point moved: from `Session` in `rho-core` to `App` in `rho-tui`.

See `SPEC-the-interactive-session-records-itself` and
`D-the-interactive-recorder-lives-on-the-app`.
