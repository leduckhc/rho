# The slash commands, driven in a real terminal

Date: 2026-08-23. Binary: `target/release/rho`, 0.1.0, default features.

Four guide pages quoted the refusal that an unbuilt slash command prints. The quote came from
a format literal in `crates/rho-tui/src/state.rs`, and nobody had watched rho print it. Every
other live check in this project used `rho run`, which has no slash commands at all.

`bench/tui_slash_probe.py` drives the real interface in a pseudo-terminal, renders the screen
with `pyte`, types each command, and reports the rows that changed. It reuses
`bench/ptyharness.py`, so teardown keeps its deadline.

```sh
python3 bench/tui_slash_probe.py
```

## The idle frame

```
ρ rho  ~/Work/Vibe/rho · main · anthropic/claude-haiku-4.5 · openrouter
                            rho · the harness, unbundled
                  anthropic/claude-haiku-4.5 · openrouter · ready
                          ❯       type a prompt to begin
                          /       list the commands
                          ?       show the keys
                          /guide  take the two minute tour
────────────────────────────────────────────────────────────────────────
❯ Type a prompt. / for commands. ? for help.
────────────────────────────────────────────────────────────────────────
  ready                                    enter send · / commands · ? help
```

This confirms the banner fields the guide names: directory, branch, model, and provider. It
confirms `ready` in the footer.

**A defect the capture found.** The fourth starter hint advertises `/guide`, and `/guide` is
not built. `STARTERS` in `crates/rho-tui/src/render.rs:721` holds it. So the first frame a new
user ever sees invites them to run a command that fails. Two of the four hints work, one is
the prompt itself, and the fourth is broken.

## The three unbuilt commands

```
--- after /model
   |✗ error · /model is not built yet. See F-slash-commands in docs/features.md.
--- after /sessions
   |✗ error · /sessions is not built yet. See F-slash-commands in docs/features.md.
--- after /guide
   |✗ error · /guide is not built yet. See F-slash-commands in docs/features.md.
```

The text matches the source exactly. The guide had the text right and the rendering wrong: the
row carries a `✗ error ·` prefix, which the pages now show.

The message also sends a user to `docs/features.md` and names a feature id. That file is an
engineering document, so the pointer is inward-facing. It is left as it is, and recorded here.

## Ctrl+O

The probe compared every screen row before and after the key:

```
--- Ctrl+O changed the screen: False
```

So the help screen lists a key that does nothing, as the guide says.

## Not covered here

- `/help` and `/quit`, which the smoke test already drives.
- The reasoning display modes, and the paste chip.
- Mouse capture, which a pty cannot exercise.
