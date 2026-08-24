# D-the-guide-is-a-paged-panel — the tour rho already promises

**Question:** `/guide` is listed in the slash list and on the first frame, and it is not
built. What shape should it take, and what content?

## The decision

`/guide` opens a **paged panel**. Three pages, `Panel::Guide(Guide { page })`, moved by the
arrows or the space bar, closed by Esc.

**Paged, and not one screen.** Two places in the product already promise "the two minute
tour": the slash list summary and the fourth starter hint on the first frame. One dense
screen does not read as a tour, and it would leave both promises half kept. A tour has a
beginning and an end, so it has pages.

**Generated where it names a key.** Every key or command the guide prints comes from the
binding table and the slash list, exactly as `F-generated-help` built the help screen. A
tour that hand-copies `ctrl-r` drifts the first time a binding changes, and the guide is the
one screen a new user trusts most.

**It names this session.** The tour prints the live model and provider, so the reader sees
their own setup rather than an example. The interface already holds both in `TuiState`.

## What this fixes on the way

The first frame advertises `/guide  take the two minute tour` as one of four starter hints,
and today that command answers an error. Driving the interface in a pseudo-terminal found it;
see `docs/verification/tui-slash-commands.md`. Building `/guide` retires the defect rather
than papering over it, so the hint becomes true instead of being deleted.

A test pins the general rule, because the next starter hint could repeat the mistake: every
starter hint that names a slash command must name a built one.

## Rules that hold

- Opening the guide never sends a prompt. No page reaches the model.
- Esc closes the panel and keeps the draft, as every other panel does. See
  `D-a-panel-nobody-can-open`.
- `/guide` always opens page one, so a second run is not a resume.
- The next key on the last page does nothing. The previous key on the first page does
  nothing.
- The footer names the page number and the key that closes. A panel that traps the eye with
  no exit is the defect `D-a-panel-nobody-can-open` already ruled out.
- **The panel owns the keyboard.** A chord does nothing while the guide is open. The review
  found that `handle_chord` runs before the panel match and guards only the history search,
  so `ctrl-r` would have swapped the panel and `ctrl-u` would have edited the draft behind
  the tour. The guard now names the guide too.
- A short screen truncates the page from the bottom, as it truncates the help. A page holds
  at most six body rows, so the loss stays small. The guide never squeezes the composer, and
  it never refuses to open, because the interface state holds no terminal height to test.

## Rules out

**Closing the panel on a next press past the last page.** It reads as friendly and it hides
a state change behind a key that meant "more". A user who presses space twice at the end
would lose the panel and wonder why. The footer carries the exit instead.

**Content in a static string per page.** A key name or a command name is never a literal. It
is interpolated from the binding table or the slash list, so a rename reaches the tour.

Prose is a different matter, and the first draft of this decision over-reached by ruling out
static content wholesale. The review showed why that is wrong: if every row is generated, then
every key-shaped token in a page came from the table by construction, and the anti-drift test
can never fail. A test whose failure the design already prevents measures nothing. So the rule
is narrow. Prose may be a literal. A name may not, and the test asserts that a key row carries
the binding's own summary as well as its keys.

**A tour in the transcript.** Rows would mix documentation into the conversation, could not
be dismissed, and would repeat on a second run.

**A page count that a config key sets.** The tour is product copy, not configuration.

**Teaching `/` and `?` by changing them.** Both already work. The guide names them; it does
not touch their behaviour.

## Extension point

`guide_pages` returns `Vec<GuidePage>`, built inside `rho-tui`. A frontend that is not the
terminal does not use it. Nothing outside `rho-tui` reads the panel, so no other crate
changes. The page list stays private product copy, and a third party that wants another tour
ships another frontend.

## Cost

One new panel variant, one key handler, one renderer, one content builder, one footer hint,
and the twenty-six tests the spec names. No change to `rho-core`, and no change to any
provider.

Four edits reach shared code in `rho-tui`, and each is admitted in the spec rather than
discovered during implementation. `footer_hints` returns an owned hint, because a page number
cannot be static. `panel_demand` and `panel_lines` each gain an arm, and the spec states the
choice each makes. `SlashCommand` gains a `built` flag, which the starter-hint test needs and
which lets the slash list mark an unbuilt command the way the help screen already marks an
unwired key.

`guide_page_count` is cut. It was a second source for a number `guide_pages` already owns.
