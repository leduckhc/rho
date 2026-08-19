# Verification: a startup notice reaches the screen

Date: 20260819. Branch `feat/tui-alternate-screen`. Built with
`cargo build --release -p rho-cli`.

This file records what was run, not what was intended. The defect it closes is
`D-a-notice-reaches-the-transcript`.

## The defect, measured before the fix

rho printed its startup notices, and then opened the alternate screen over them. A pty
harness read the raw byte stream and found the order.

```
alt_enter_at: 535 | no-model warning at: 5 | skill warning at: 320
=> warnings printed BEFORE alt screen: True
```

So each notice was on the primary screen for a few milliseconds. The user never reads that
buffer again. One hidden line says a project skill stays unloaded until the user trusts it.

## The rendered screen after the fix

Captured from the release binary through a pty at 30 rows by 100 columns, replayed through a
terminal emulator, and read from the alternate screen buffer.

```
ρ rho  ~/Work/Vibe/rho-altscreen · feat/tui-alternate-screen · anthropic/claude-haiku-4.5 · openrout

                                                ▄▀▀▄
                                                █  █
                                                █▄▄▀
                                                █

                                    rho · the harness, unbundled

                          anthropic/claude-haiku-4.5 · openrouter · ready

                                  ❯       type a prompt to begin
                                  /       list the commands
                                  ?       show the keys
                                  /guide  take the two minute tour

! notice · no model given, so using the default for openrouter:
           anthropic/claude-haiku-4.5. Set --model or RHO_MODEL to choose
           another.
! notice · 10 skills: the frontmatter sets allowed-tools. rho ignores this
           field. The approval policy stays the only authority. Affected:
           agent-browser, diagrams-as-code, firecrawl, and 7 more.
! notice · 1 project skill(s) were found and not loaded: tui-design. A skill can
           instruct the model and can carry scripts, so a skill from this
           repository stays off until you trust it. Pass --trust-project to load
           them.
────────────────────────────────────────────────────────────────────────────────────────────────────
❯ Type a prompt. / for commands. ? for help.
────────────────────────────────────────────────────────────────────────────────────────────────────
  ready                                                           enter send · / commands · ? help
```

Three things to read from that frame. The splash survived. Every notice wraps, so
`Pass --trust-project to load them.` is on screen. And the notice text is not an error.

## Every path, including the failure paths

`primary_leak` counts the bytes rho writes before it opens the alternate screen. Zero is the
requirement, because anything there is a line the user cannot read.

```
1st run, default                   exit=0 alt=1 notice_rows=3 primary_leak=0b
2nd run, same again (twice)        exit=0 alt=1 notice_rows=3 primary_leak=0b
--trust-project                    exit=0 alt=1 notice_rows=2 primary_leak=0b
explicit model                     exit=0 alt=1 notice_rows=2 primary_leak=0b
--no-mouse                         exit=0 alt=1 notice_rows=3 primary_leak=0b
2-row terminal (fatal path)        exit=1 alt=0 notice_rows=0 primary_leak=53b
    (no alt screen; stderr said: 'rho: the terminal is 2 rows, and rho needs at least 4')
azure, no default model (error)    exit=1 alt=0 notice_rows=0 primary_leak=136b
    (no alt screen; stderr said: '... Set --model or the RHO_MODEL variable. For azure, the
     value is your deployment name.')
```

The counts are correct, one by one:

- `--trust-project` drops the skill notice, so two remain.
- An explicit model drops the model notice, so two remain.
- The two failing runs never open the screen, so stderr is the right channel. A leak there is
  the wanted behaviour, not a defect.
- Running it twice gives the same result. "Twice" has caught two defects in this project.

## Breaking it on purpose

Each new test was run against a deliberate break, and each one failed. The files were copied
to `/tmp` first, never restored with `git checkout`.

| The break | The test that failed | What it reported |
| --- | --- | --- |
| `with_notices` accepts the notices and drops them, as before | `the_app_seeds_its_notices_into_the_transcript` | `left: []`, right held both notices |
| A notice draws with the `✗ error` glyph | `a_notice_row_draws_its_text_and_says_notice` | the row claimed an error |
| The splash folds notices in with no fit check | `many_notices_stay_reachable_instead_of_truncated` | 40 notices reported no scrollable rows |
| A notice pads to one line instead of wrapping | `a_long_notice_keeps_its_tail` | the tail was missing from the drawn rows |

Two tests were wrong when first written.

`a_long_notice_keeps_its_tail` asserted one exact phrase, and a wrap legitimately split it
across two rows. The assertion now rejoins the drawn rows and compares the whole notice, so a
clip anywhere fails. That is a stronger test, not a weaker one.

`a_notice_is_not_an_error` only asserted that the first row was **not** an error. So it also
passed when `push_notice` pushed nothing at all. A review found it. It now proves the notice
exists, that it is the only row, and that no error row appeared.

## The review, and the defect it found

A reviewer read the commit and asked one question this project has learned to ask: does
another defect of the same family exist? It did.

**Every render test used width 100.** The wrap width was `measure - head`, where `measure` is
`min(80, width - 10)` and the label `! notice · ` is 11 columns. So at width 24 or less the
wrap width reached zero, `wrap` returned one empty line, and **the whole message vanished**.
Only the label drew. That is the defect this file exists to close, at a narrower size, and a
split pane or a phone over ssh reaches it.

Measured before the fix, asking whether the action `--trust-project` survived:

```
width= 16 keeps the action: false
width= 20 keeps the action: false
width= 21 keeps the action: false
width= 22 keeps the action: false
width= 24 keeps the action: false
width= 30 keeps the action: true
```

The reviewer estimated the bound at 21. Measuring put it at 24, so the estimate was optimistic
and the measurement decided. `NOTICE_MIN_TEXT` now sets a floor of 12 columns for the text. Below
it the label takes its own row and the text takes the whole measure, because the text is the part
that matters. At width 24:

```
|! notice ·              |
|1 project               |
|skill not               |
|loaded. Pass            |
|--trust-project         |
|to load them.           |
```

After the fix, every width from 16 to 120 keeps every word. `a_notice_survives_a_narrow_screen`
sweeps widths 20 to 120 and asserts no word is lost. Setting `NOTICE_MIN_TEXT` back to 0
restores the defect, and that test fails.

## The gate

```
cargo fmt --all --check                                              clean
cargo clippy --workspace --all-targets --all-features -- -D warnings 0 warnings
cargo test --workspace --all-features                                827 passed, 0 failed
cargo build -p rho-cli --no-default-features --features minimal       ok
python3 bench/check-ids.py                                            VIOLATIONS 0
python3 bench/check-prose.py $(find docs -name '*.md')                 VIOLATIONS 0
```

The test count was 815 before this work and 827 after, so twelve tests are new.

`bench/check-ids.py` earned its place here. Retiring `F-inline-band`, `F-freeze-upward`, and
`F-optional-mouse` left nine dangling references in two older specs and in the progress
ledger. The guard found every one. The rows now stay as `superseded`, each naming its
replacement, because a dangling reference is worse than a history note.

## The layout tests, and the one that was vacuous

`plan_screen` is public and had no direct test. Ten tests now cover it, in
`crates/rho-tui/tests/layout.rs`. Each was run against a deliberate break.

| The break | Tests that failed |
| --- | --- |
| `height < STARTUP_MIN_ROWS - 1`, an off-by-one on the minimum | 3 |
| `MAX_DRAFT_ROWS` raised from 10 to 12 | 2 |
| `let banner = true`, so the banner draws without paying a row | 4 |
| `let floor = 0 * panel_floor`, so the panel floor is gone | **0, at first** |

The fourth break passed every test. `a_panel_floor_survives_a_tall_draft` asked for 20 rows,
and at 20 rows the panel gets its whole want of 6, so the floor decides nothing there. The
test was vacuous, and it would have passed for the life of the project.

A probe printed the panel height for every terminal height from 4 to 29. The floor only binds
between 8 and 16 rows, where the ten-row draft would otherwise squeeze the panel out. The test
now sweeps that band and asserts the panel holds exactly its floor. It then fails against the
fourth break, as it always should have.

This is the third time in this project that a test passed against the bug it was written for.
The lesson holds: a test is not evidence until the break has been watched.
