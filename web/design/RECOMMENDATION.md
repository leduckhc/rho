# Recommendation — getrho.dev home page

Status: decided. The controller confirmed this direction as decision D-004
in `.rho-work/DECISIONS.md`. Do not re-open it.

## 1. The winner

The winner is mockup C (`mockup-c-parts.html`), with one element from
mockup A: the `$ <command>` eyebrow label above each section.

Reasons:

- The crate picklist is the hero. The `[x]` and `[ ]` boxes state
  "you pick the parts" in one glance. That is the most important message
  on the page. Mockup A only lists the crates.
- C mixes a proportional face for headings with mono for code. That gives
  a real type hierarchy. A sets body text in mono at full width, which
  tires the reader.
- A's `$ cat crates.txt` frame is a costume. The content is not a terminal
  session, so the frame fights the content.
- A's `$ <command>` eyebrow labels are terminal-native without that costume.
  Use them in place of C's all-caps kickers.

What building the mockups taught us, which a screenshot does not show:

- The comparison table needs `min-width` plus an `overflow-x: auto` wrapper.
  Without both, the table crushes at 390 px or overflows the page.
- Numeric cells need `font-variant-numeric: tabular-nums` and right
  alignment, or the memory column reads as noise.
- The one-screen-per-idea layout survives 390 px with only two changes:
  smaller display sizes and single-column tier cards.
- The sticky header must repeat the page background. A transparent sticky
  header makes table rows bleed through it on scroll.

## 2. Page outline for the build stage

Implement these sections in this order. Each section shows its `$ <command>`
eyebrow label in mono, `--fs-caption`, `--accent`.

1. Eyebrow `$ rho --about`. Heading: "rho is the harness, unbundled."
   Copy intent: state what rho is in one sentence, then the composability
   claim in three short sentences. Data: none.
2. Same screen, below the lede: the crate picklist table from README.md,
   with the `[x]`/`[ ]` example-selection column and the accessible-label
   pattern from section 4. Data: all ten crates with roles. Footnote: every
   crate except `rho-core` is optional; no crate depends on a frontend.
3. Eyebrow `$ man rho`. Heading: "You do not fork rho. You compose it."
   Copy intent: contrast the bundled-harness model with rho's trait model.
   Data: the minimal-build command,
   `cargo build -p rho-cli --no-default-features --features minimal`.
4. Eyebrow `$ rho bench --sessions`. Heading: "A session should cost
   megabytes, not hundreds." Copy intent: give the measured reason rho
   exists, and stay honest about unmeasured rho values. Data: the
   comparison table from BRIEF.md section 2. The rho row renders
   `to be measured` in italic `--muted` and links to `docs/benchmarks.md`.
   The attribution line names jcode.sh and the sample date, August 2026.
5. Eyebrow `$ rho extend --help`. Heading: "Extend it in Rust, or in any
   language." Copy intent: the two-tier extension story in two cards.
   Data: Tier 1 compiled traits (`Provider`, `Tool`, `Hook`); Tier 2
   out-of-process plugins over stdio JSON-RPC, crash-isolated.
6. Eyebrow `$ rho install`. Heading: "Install from source. MIT licensed."
   Copy intent: give the install command and the two exits. Data:
   `cargo install --git https://github.com/leduckhc/rho rho-cli`, a GitHub
   button, a Docs link.
7. Footer: licence, GitHub, Docs, and the sentence "Every number on this
   site links to the test that produced it."

Per D-003: no rho performance number appears anywhere until S11 fills
`docs/benchmarks.md`.

## 3. What is deliberately left out

- No screenshot wall. jcode.sh proves it adds length, not conviction.
- No changelog on the home page. GitHub releases already hold it.
- No benchmark wall. One honest table carries the whole claim.
- No web fonts. System fonts cost zero bytes and cause no layout shift.
  The site must embody the small-footprint claim.
- No JavaScript framework, and no runtime JavaScript on the critical path.
- No gradients, no feature-card grid of icons, no hero illustration.
  These are the generic patterns the design must avoid.
- No pricing, no star chart, no testimonials. rho is a library, not a SaaS.

## 4. Carry over from DESIGN.md verbatim

The build stage must copy these without re-deciding them:

- The full dark and light token sets in DESIGN.md section 2, hex for hex.
- The two font stacks in section 3. No additions.
- The type scale, the 68 ch measure, and the 880 px shell. Note: mockup C
  uses a 1080 px shell for its full-screen sections; keep C's 1080 px for
  the home page and reserve 880 px for future doc pages.
- The spacing scale and the three radii in section 4.
- The code-block and table treatments in sections 5 and 6, including the
  3 px `--accent` left border on the rho row.
- The interactive states table in section 7, including the focus outline.

## 5. Accessibility floor, do not regress

- Body text on page background: 15.30:1 dark, 16.56:1 light.
- Muted text on page background: 8.57:1 dark, 8.69:1 light.
- Accent on dark background: 8.97:1. Links on dark: 7.81:1 or better.
- `--faint` (5.98:1 dark, 5.81:1 light) sets captions only, never body.
- Light `--accent-ink` on `--accent` is 3.63:1. Buttons in light mode must
  render at 17 px bold or larger.
- Picklist pattern, chosen and now present in the mockup: the `[x]` glyph
  carries `aria-hidden="true"`; a visually hidden `<span class="sr-only">`
  in the same cell reads `included` or `not included`; the first column
  header holds hidden text `In this build`; the table has an `aria-label`.
  We chose this over a disabled checkbox because the column is a static
  example, not a control, and the pattern changes no visible pixels.
- Keep the skip link, the single `<h1>`, the labelled `<nav>`, and the
  landmark structure of the mockup.
- Tables keep captions or `aria-label`s, and scroll horizontally at small
  widths inside an `overflow-x: auto` wrapper.
