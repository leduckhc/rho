# The Ledger band, driven for real

Date: 20260818. Binary: `cargo build --release -p rho-cli`.
Command: `./target/release/rho --provider openrouter --model anthropic/claude-haiku-4.5`
Terminal: ghostty, 100 by 30, inside tmux.

This records what a real run showed after the renderer took the Ledger design. See
`D-ledger-wins-the-band` and `docs/design/tui-mock.html`.

## 1. The help screen now draws its keys

It drew **no rows at all** before. `help_panel` built one row for each of the 22 bindings,
which is 27 rows with its chrome, and `plan_band` granted the panel all of its rows or
none. A 14-row band could never hold 27, so the panel was dropped whole. The frame fixture
`100-help.txt` pinned that blank output as correct.

```
  keys  1-8 of 22                                                          ↓ 14 more below
  enter          send the draft
  shift+enter    insert a newline, where the terminal reports the key
  ctrl-j         insert a newline, in every terminal
  alt+enter      insert a newline, in every terminal
  ctrl-c         cancel the turn · press twice while idle to quit
  ctrl-d         quit while the draft is empty
  ctrl-r         search the history
  ctrl-x ctrl-e  edit the draft in the editor
────────────────────────────────────────────────────────────────────────────────────────
❯ █
────────────────────────────────────────────────────────────────────────────────────────
  ready                                        ↑ ↓ scroll · esc close · / lists commands
```

The count comes from `bindings()`, so it cannot drift from the table.

## 2. The arrows answer the promise, after a live run found they did not

The first live run drew `↓ 14 more below` and offered `↑ ↓ scroll`, and **the arrows did
nothing**. `handle_help_key` answered only `esc` and `?`. That is
`D-a-panel-nobody-can-open`, and this change introduced it. No test saw it, because the
tests asserted the marker and never pressed the key.

Six presses of the down arrow, live:

```
  keys  7-14 of 22                                                               ↑ 6 · ↓ 8
```

Twenty presses, live. The window stops, and it makes no false claim about rows below:

```
  keys  15-22 of 22                                                        ↑ 14 more above
  esc            close a panel · press twice to clear the draft
```

## 3. A real turn, with the new row grammar

```
❯ read Cargo.toml and say the workspace member count only
  ✓ read                                                                                0s
The workspace has 1 member (using glob pattern `crates/*`).
────────────────────────────────────────────────────────────────────────────────────────
❯ Type a prompt. / for commands. ? for help.
────────────────────────────────────────────────────────────────────────────────────────
  done · end turn · 1.4s                                  enter send · / commands · ? help
```

The status glyph leads, the row indents two columns, and the duration sits flush right.

## Defects this run found, and their state

### A. A tool row states no payload. Open.

The row above reads `✓ read` and never says **which file**. The mock states
`✓ read  crates/rho-tui/src/render.rs · 220 lines`, so the grammar matches and the content
does not.

Cause: `state.rs` builds the row from `StreamEvent::ToolCallEnd` with
`preview: String::new()` and `kind: ToolKind::Other`. The tool arguments never reach the
row, so no fixture could catch it. Every frame fixture supplies a payload by hand, and the
product never produces one.

`ToolKind::Other` is also the fail-open variant that `D-plugin-does-not-classify-itself`
warns about, and it is hard-coded here for every tool.

This is a verified defect and it is **not fixed**. The fix plumbs the tool arguments from
the core event into the row, which is a larger change than the band layout, so it belongs
in its own stage with its own tests.

### B. `shift+enter` cannot reach rho. Open.

`shift+enter` inserts a newline in the handler, and a live run showed it **sending the
draft** instead. rho calls `enable_raw_mode` and never pushes the keyboard enhancement
flags, so the terminal reports a bare carriage return and the shift is invisible. A probe
of `supports_keyboard_enhancement` reported `not supported` under tmux.

No fixture could catch this either. Every test synthesises a `KeyEvent` that carries
`KeyModifiers::SHIFT`, and no terminal sends one today.

The binding table now states the dependency: `shift+enter` reads
`insert a newline, where the terminal reports the key`, and `ctrl-j` reads
`insert a newline, in every terminal`.

## The gate at this commit

```
cargo fmt --all --check          clean
cargo clippy … -D warnings       0
cargo test --workspace           817 passing, 0 failing
cargo build -p rho-cli --no-default-features --features minimal   ok
python3 bench/check-ids.py       VIOLATIONS 0
python3 bench/check-prose.py     VIOLATIONS 0
node docs/design/check-mock.js docs/design/tui-mock.html   13 frames, VIOLATIONS 0
```
