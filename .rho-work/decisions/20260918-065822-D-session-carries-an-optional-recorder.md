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
