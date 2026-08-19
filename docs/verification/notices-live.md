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

## Block text, and why the sanitiser stayed

The owner said the line sanitiser is wrong for a coding agent and asked to remove it. The
fault was real. The removal would have been too wide.

Measured before the fix, with a fixture that matches what a model sends:

```text
model sent:  Intro paragraph.\n\n- alpha: first\n- beta: second\n\n```rust\nfn main() {}\n```
rho drew:    Intro paragraph. - alpha: first - beta: second ```rust fn main() {} ``` Done.
```

`rho_redact::sanitize_text` already keeps `\n` and `\t` and already drops every escape. The
whole defect was `sanitize_line`, a four line wrapper that folds a newline into a space. So
block text now calls `sanitize_block`, and single-line rows keep the wrapper.

Two deliberate breaks, and each failed the tests it should:

| The break | Tests that failed | Why |
| --- | --- | --- |
| Block text back on `sanitize_line` | 4 | the structure collapses again |
| **No sanitiser at all**, as first asked | 2 | `\x1b[2J` and an OSC 52 clipboard write reach a terminal cell |

The second row is the reason the filter stayed. A cell holding an escape is written to the
terminal and the terminal obeys it, so untrusted model output could clear the screen, move
the cursor to draw a fake approval prompt, or write the user's clipboard.

Verified live afterwards, against openrouter, asking for a list and a nested code fence:

```
- Item one
- Item two

```rust
fn main() {
    println!("hello");
    if true {
        if true {
            println!("nested");
        }
    }
}
```
```

The bullets take their own rows, and the indent is right at four, eight, and twelve columns.

## The screenshots, and a tool that lied

`ttyd` served the interface over HTTP, and a real browser rendered it. Input came from
`tmux send-keys`, so every keystroke was deterministic, and each frame was checked against
`tmux capture-pane` before it was captured. No screen-recording permission was needed.

`vhs` was tried first, and its `Screenshot` command produced two frames that showed defects
that do not exist: a chopped logo over the help panel, and a slash list that appeared not to
open. Both were reproduced at vhs's exact terminal size, 50 by 143, measured with `stty` and
not guessed. rho was correct in both. The command grabs a frame that can precede the
repaint. The continuous recording is clean.

A screenshot tool is a measuring instrument, and this one needed calibrating before it could
be trusted.

## Markdown as colour, verified live

Driven against openrouter, asking for a heading, prose, a bullet list, a quote, a rule, and a
nested code fence. Captured through `ttyd` in a real browser.

```
Results                                     <- green, bold, no hashes
This response demonstrates the requested formatting elements in a structured
layout.                                     <- body text
• Item one with example content              <- accent
• Item two with additional detail
┃ This is a blockquote containing relevant information.    <- dim
────────────────────────────────────────────────────────   <- muted rule
```rust                                      <- dim fence
if condition {                               <- blue code
    if nested_condition {
        println!("nested code");
    }
}
```
```

The screenshot is `shots/7-markdown-colour.png`. Nesting is right at four and eight columns,
and no markup punctuation reaches the screen except the fence, which keeps its markers on
purpose, as pi draws it.

### Two more tests that proved nothing, and what they cost

This phase went through four deliberate breaks. Two of them tripped nothing at first.

**An alignment row test was too narrow.** A scanner that trimmed the outer pipes before
testing for a rule still passed, because the test only used `|---|---|` and `| --- | --- |`.
A one-column `|---|` catches it, and the test now includes one.

**A padding test could never fail.** It counted cells whose symbol was "not empty", and an
untouched ratatui cell holds a space, not an empty string. Rewritten to draw a long answer and
then a short one into the same terminal, it still did not fail with the padding deleted, and
that is the useful part: **the premise was wrong.** `Terminal::draw` resets its back buffer and
emits a diff, so a short row already clears its own tail. Padding a row is a consistency
convention here, not a correctness guard. The test was deleted and the claim with it, because a
test that cannot fail is worse than none.

That is the fourth and fifth vacuous test caught in this branch, all by the same method: break
the code where the rule actually binds, and watch.

## Measured against pi and jcode, in a browser

All three were run in `ttyd` and given the same prompt, on the same model,
`anthropic/claude-haiku-4.5` through openrouter. Their colours were read from their own escape
codes with `tmux capture-pane -e`, so this is measurement and not a reading of their source.

| Element | pi | jcode | rho, phase 1 |
| --- | --- | --- | --- |
| heading | bold, RGB 240,198,116 | bold **and underlined**, RGB 240,190,90 | bold, accent |
| bold | modifier `1` | modifier `1` | **markers show** |
| italic | modifier `3` | modifier `3` | **markers show** |
| inline code | RGB 138,190,183 | RGB 140,180,255 **on a background**, RGB 45,45,45 | **backticks show** |
| list glyph | `-`, muted | `•`, RGB 100,100,100 | `•`, body colour |
| list text | body colour | body colour | body colour |
| blockquote | italic, grey, a bar | grey, a bar, plus action badges | dim, a bar |
| rule | grey | grey | muted |
| code block | highlighted, indented two columns | **a bordered box with a language header**, highlighted | one colour |
| table | box borders | aligned columns, bold header, a divider | **verbatim pipes** |

The screenshots are `shots/8-pi-reference.png`, `shots/10-jcode-reference.png`, and
`shots/9-rho-same-prompt.png`.

**Where rho matches both.** The heading, the rule, the fence, the list glyph, and the list
text. The list text was the one thing the comparison changed: rho coloured a whole item with
the accent, and both tools leave item text at the body colour. rho now does too.

**Where rho does not match either.** Inline code, bold, italic, syntax highlighting, and
tables. The first three are phase 2, and they are the ones a reader notices, because the
punctuation is still on screen.

**What phase 2 should take from jcode.** A background behind inline code, RGB 45,45,45 under
RGB 140,180,255, reads more clearly than a foreground change alone. It also costs a background
colour in the role table, which the current `RoleStyle` has no field for. That is a contract
question for phase 2, not a detail.

**A claim here was corrected.** An earlier note said pi "refuses to guess a language" and
implied rho matches it. pi refuses to guess, and it does highlight when the language is named,
which `rust` was. Both tools highlight. rho does not, and the gap is real.

**Two things the run itself taught, neither about rendering.**

Asked for markdown, haiku wrapped its whole answer in a markdown fence. rho drew all of it as
code, correctly, and the answer looked unstyled. pi and jcode would do the same. A fence means
code, even when a model wraps a whole message in one. The comparison had to say "do not wrap it
in a code fence" to be fair.

jcode's Bedrock default, Claude 3.5 Sonnet v2, is end of life and returns a 404 with "This
model version has reached the end of its life". Its one-key fallback then offered 3.5 Haiku,
which is also end of life, so the fallback ping-ponged between two dead models. Worth knowing
before rho copies a fallback: a fallback list needs a liveness check, or it trades one dead
model for another.

## Inline styling, shipped and verified live

Phase 2 is in. The same prompt, the same model, and no marker reaches the screen.

```
Heading Two                                        <- bold, accent, no hashes
This sentence contains bold text, italic text, and inline code all together.
                          ^^^^^^^^  ^^^^^^^^^^^      ^^^^^^^^^^^
                          bold      italic           code colour
• The first item has bold content and code snippet inside
• The second item also contains bold and another code block here
┃ This is a blockquote line with emphasis.
────────────────────────────────────────────────────────────────
```rust
fn main() {
    let x = 5;
    if x > 3 {
```
```

The screenshot is `shots/11-inline-styling.png`. No `**`, no `*`, and no backtick is on screen.

### The contract, and where the width invariant lives

A row was `(String, Style)`, one style for the whole row. It is now `StyledLine`, a list of
styled runs, and 32 producer sites were converted. Review asked whether `StyledLine` should be
a struct owning its width. It is not. **`put` owns it**, and `put` is the only consumer: it
clips a run at the right edge and pads a short row. One place, in code, for every producer
including a future one. A struct would spread the same rule across 32 call sites and still rely
on each of them calling it.

Wrapping now runs **over runs**, not over text, which is what closes the ordering bug review
found. A per-character pass carries each character's style, then neighbours of one style are
coalesced back into runs.

### Two rules where rho deliberately leaves CommonMark

Both come from the domain, and both have tests.

**An underscore never carries emphasis.** CommonMark renders `__init__` as bold. rho leaves it
alone, because in a coding agent's prose an underscore is an identifier: `wrap_block`,
`snake_case`, `__all__`, `_private`. Only `*` carries emphasis.

**An intraword star never opens.** CommonMark italicises the `3` in `2*3*4`. rho requires a
non-alphanumeric before an opening marker and after a closing one, so multiplication and globs
survive.

### The breaks, and two that taught something

| The break | Result |
| --- | --- |
| Wrap over raw text instead of runs, the ordering bug | 4 tests fail |
| Let an underscore open emphasis again | 2 tests fail, `__init__` is eaten |
| Remove the closing flanking rule only | **nothing fails** |
| Remove the intraword rule only | **nothing fails** |
| Remove flanking entirely, the naive rule | 1 test fails: `2 * 3 * 4` becomes `2  3  4` |

The two that failed to trip are the interesting ones, and they are not vacuous tests this time.
Each single rule is covered by another: for `2 * 3 * 4` the opening rule catches it, and if that
is removed the closing rule does. Removing one leaves the other standing. Only removing flanking
altogether gets through, and the test catches that, reporting exactly the `2  3  4` the review
predicted before any code existed.

That is defence in depth rather than a hole, but it took three attempts to establish which, and
the difference matters: a break that trips nothing is either a weak test or a redundant rule,
and only tracing it says which.

## Tables and coloured emphasis, verified live

The owner asked for two more things: emphasis visible by colour and not only by a modifier, and
tables, "since an LLM very often outputs tables".

Both are in. Drawn by the release binary against openrouter, at 156 columns:

```
Rust Ecosystem Overview

Here is a comprehensive guide using the tokio runtime for async operations.
             ^^^^^^^^^^^^^^^^^^^        ^^^^^
             emphasis, coloured         code colour

Crate │ Purpose                       │ Status  │ Downloads
──────┼───────────────────────────────┼─────────┼──────────
tokio │ Async runtime and utilities   │ Active  │     1.2M+
serde │ Serialization framework       │ Stable  │     v1.0+
axum  │ Web framework                 │ Growing │      500K+
```

The screenshot is `shots/12-tables-and-inline.png`. No pipe, star, or backtick reaches the
screen. The `Downloads` column is right aligned, from the model's own `---:` marker.

**Emphasis now carries a colour as well as a modifier.** A modifier alone is not enough: many
terminals draw no italic at all, and some draw bold at the same weight, so the emphasis would
vanish. Bold reads brighter and italic reads warmer, so the two are told apart. Inside a heading
or a quote the modifier carries it alone, because those rows already own a colour and repainting
a word inside one looks like a defect.

**A cell's markers come off before its width is measured.** Measuring `**bold**` and removing the
stars later would shift every column to its right. The cost is stated rather than hidden:
emphasis inside a cell is dropped, not styled, because a row here is one string and cannot carry
runs per cell.

### One test was superseded, on purpose

`a_table_degrades_to_verbatim_text` asserted that every table row stays plain text. That was
right while rho drew no tables, and it came from the contract review's warning that half a table
drawn is worse than none. rho draws them now, so the assertion contradicts the feature.

The warning still holds, and it is now guarded by two tests instead of by not having the feature:
`a_table_without_a_rule_row_stays_verbatim` and `a_table_inside_a_fence_stays_code`. A table
draws only when it is unambiguous.

### The breaks, and two more weak tests caught

| The break | Result |
| --- | --- |
| Accept any two pipe lines as a table | 1 test fails |
| Drop a ragged row's missing cells | 1 test fails |
| Measure a cell's width with its markers still in | **nothing failed at first** |
| Ignore the alignment markers | **nothing failed at first** |

Neither of those two was exercised. The table in the tests had no inline markup in any cell, so
the third break changed nothing at all. And the fourth was masked by a `trim_end`: with the text
trimmed, a left aligned `7` and a right aligned `7` both end the row, so the assertion held
either way.

Both tests were rewritten. One table now carries `**bold**` and `` `code` `` in its cells and
asserts the columns still line up. The alignment test now compares display columns, and checks
both that a right aligned cell sits at the far edge and that a left aligned one sits one pad
after the divider. Both breaks then fail.

That is the sixth and seventh weak test caught on this branch by the same method. The method is
cheap and it keeps paying: write the break that the rule must catch, and watch.

### A byte offset is not a column

`the_columns_align_across_every_row` failed against correct code, because it measured alignment
with `str::find`, which returns a byte offset. A rule glyph is three bytes, so the rule row
reported 27 where the header reported 9. Alignment is a display property and is now measured in
display columns. The same mistake was made once before on this project, in the frame fixtures.

## A blockquote leans

The owner asked for an italic blockquote. pi italicises one and jcode does not, so this follows
pi. Verified live on Bedrock: both lines of a two-line quote lean, stay dim, and keep the bar. The
screenshot is `shots/14-italic-blockquote.png`.

The change went into the role table and not into the renderer. `RoleStyle` had `color`, `dim`,
`bold`, and `reversed`, so a lean had nowhere to live and `MdItalic` was faking one with
`reversed` in the no-colour mode. `RoleStyle` now has an `italic` field, `style_for` reads it, and
`MdItalic` states a real lean instead of a stand-in.

`MdQuote` also returned as its own role. It was folded into `Muted` when a quote was only quiet,
and a quiet lean is a meaning no other role carries. The rule stays the same as when six roles
were trimmed to two: a role exists only when no current role says the thing.

## Text fills the width

The transcript wrapped at `min(80, width - 10)`, so a 156 column terminal used about half its
screen. It now wraps at `width - 1`. There is no margin on either side.

**One column is reserved, and only one.** The scroll rail draws at `width - 1` as an overlay on the
transcript, so text using the whole width lost its last character to the rail whenever the
transcript overflowed, which is the normal state of a session.

Reserving that column only while the rail shows is circular, and the cycle is worth naming: the
measure decides how many rows the text wraps to, the row count decides whether it overflows, and
the overflow would decide the measure. A layout that depends on its own output flickers at the
boundary. So the column is reserved always. See `D-text-fills-the-width`.

Four tests pin it: text wider than 120 columns on a 156 column frame, the rail never overwriting a
character, a horizontal rule filling every column, and a 20 column terminal still showing every
word.

### The fixtures were regenerated, and the diff was read first

Four design frames changed. The diff was checked before staging, and every change is a line holding
more words. Row counts stay at 24 and the chrome is untouched. At 40 columns the measure went from
30 to 39, which is the clearest gain:

```
-❯ now bind the session clock
-  to the header slot
+❯ now bind the session clock to the
+  header slot
```

Two of the four new tests failed on their first run, against correct code, because they measured
**every** row including the composer rules and the footer. Those are chrome and fill the frame by
design, so one reported a widest row of 156 and one failed on a full-width divider. Both now filter
to the transcript rows they are about.

## A submitted prompt sits on a band

The owner asked for a full-width background behind an already-submitted user message, and sent
screenshots of Claude Code and pi doing it. Verified live on Bedrock, at 156 columns, with one short
prompt and one that wraps:

```
❯ Say only: first answer                                      <- banded, full width, bold
first answer                                                  <- no band

❯ Now write a longer prompt that will wrap across more than one line so I can see whether the
  only: second answer                                         <- both rows banded
second answer                                                 <- no band
```

The screenshot is `shots/18-user-band.png`. The band reaches the frame edge on every row, a wrapped
prompt bands all of its rows, and no answer is banded.

**A background is a fourth theme mapping.** `RoleStyle` describes the 16-colour and no-colour modes,
and neither can carry a quiet background: there is no subtle grey in 16 colours and no colour at all
in the third mode. So `role_bg_256` is its own mapping, one role uses it, and the theme test now
fails if a second role takes a background.

**One mechanism carries the band to the edge.** The producer does not pad the row. `put` fills a
row's tail with the row's own style, and that is what makes the band full width. Padding at the
producer as well would have worked and would have left that fill untested.

### A break that did not trip, and what it changed

Three breaks were run. Turning the band off failed three tests, and giving a second role a
background failed the theme test. **Filling the tail with the default style failed nothing.** The producer was padding the row to the
width as well, so the fill in `put` never ran for a banded row. Two mechanisms, one of them dead.

The producer's padding was removed rather than the fill, so the band now depends on the fill and the
break fails two tests. That is the better shape: one path, exercised.

### The weight was a side effect, and is now a decision

The banded text also drew bold. `style_for` reads modifiers from the 16-colour table whatever the
colour depth, so the `bold` meant as the 16-colour fallback also applied beside the band in 256
colours. It was noticed in the live screenshot, not in a test.

It reads well and the words are the user's own, so it was kept and pinned by
`a_submitted_prompt_is_bold_in_every_colour_mode`. A side effect that is kept without a test is a
side effect waiting to be deleted by someone tidying up.

## The last security finding: the banner was the one row with no filter

A security review listed four paths and the controller closed three in the critic-panel commit. The
fourth, `banner_line`, was missed and is closed here.

It joined the directory, the branch, the model, and the provider **raw**, while every other row had a
filter. The directory is the realistic vector: a name on a Unix filesystem may hold an escape byte, so
running rho inside a hostile checkout would put that byte on the banner. Git rejects a control
character in a ref name, so a branch is safer. A model id arrives from a flag or a config file.

**Nothing escaped, and that is the uncomfortable part.** ratatui drops an escape from a cell, so the
attack did not work. The review's sharpest observation was that **rho had two filter layers and only
one of them was rho's**. A defence that rests on a dependency's behaviour is a defence that changes
when the dependency does, and it does not travel: banner text that is logged, copied through OSC 52,
or written by a different backend would carry the escape.

Every field is filtered now. `the_banner_sanitises_every_field_it_joins` covers an escape sequence, a
bell, and a bidirectional override, and asserts the readable text survives so the filter is not a
blunt instrument. Removing the filter from the directory alone makes it fail.

## The last two test-quality findings

A test-quality audit ran seventeen mutations and named four survivors and one tautology. Three
survivors were closed in the critic-panel commit. These are the last two.

### A test that asserted nothing

`theme_resolves_every_role` called each of the three resolvers and threw the result away with
`let _ =`. The matches are exhaustive, so they cannot panic: it was a compile check wearing a test's
clothes, and it would have passed against any body that returned something.

It is replaced by the invariant the three tables exist for. **A role has to look different from body
text, in whichever mode the terminal gives us.** In 256 colours that means a foreground or a
background. In sixteen it means a colour or a modifier. With no colour only a modifier is left, so
there has to be one. `Text` is body text and is the one role that must resolve to plain.

A second test now holds that no role names an index below 16, because those are the terminal's own
sixteen and a user theme redefines them, so a role that named one would change meaning per terminal.

Both were checked by breaking them: removing the dim from the code role in the no-colour mode reports
`role MdCodeBlock is invisible with no colour`, and moving a heading to index 10 fails the range test.

### A clip two reviews disagreed about

The audit called the run clip in `put` dead code: `set_stringn` clamps to the buffer edge by itself,
so no mutation of the clip changes an observable cell, and no test can pin it. By the audit's own
rule, untested code should go.

The security review found the opposite failure the same day, in `banner_line`: rho was leaning on
ratatui to drop an escape and calling that a defence. A guarantee that lives in a dependency changes
when the dependency changes, and it does not travel to a log, a clipboard write, or another backend.

**The clip stays, for the second reason, and the code now says so.** It costs one comparison per run
and it makes `put`'s promise true in rho's own code. The comment states plainly that this is defence
in depth and not a tested guarantee, because the one honest thing to avoid here is a claim that a test
backs it.
