# rho design system — getrho.dev

This file defines the design system for the rho website. A developer can
implement the site from this file alone. Dark mode is the default. Light
mode follows the same token names.

Evidence base: we extracted the real stylesheet of `https://jcode.sh`
(`styles-89682ca7077eeeca.css`, fetched 2026-08-17). jcode uses a monochrome
palette (`#111/#666/#999/#ccc/#f4f4f4/#fff`), self-hosted JetBrains Mono,
an 860 px measure, line-height 1.65, and one 8 px radius. Its styling is
restrained; its length is not. We keep the restraint and the evidence-first
tables. We cut the length. The retired `agentsdk.build` (Wayback capture
2026-01-28) shows the library-first framing: install command first,
comparison table, terminal aesthetic. We keep that framing.

## 1. Principles

- The site is the product claim. rho says it is small. The site must be small.
- Zero web fonts. Zero runtime JavaScript on the critical path.
- Every number is measured or marked `to be measured`. No exceptions.
- Dark mode is the default. Developers read terminals; meet them there.

## 2. Color tokens

Contrast ratios are computed with the WCAG 2.1 relative-luminance formula.
The floor is AA (4.5:1 for body, 3:1 for large text and UI parts). Body text
clears 7:1 in both modes.

### Dark mode (default)

| Token | Hex | Role | On background | Ratio |
| --- | --- | --- | --- | --- |
| `--bg` | `#101318` | page background | — | — |
| `--surface` | `#181c23` | cards, code blocks | — | — |
| `--surface-2` | `#232833` | inline code, hover rows | — | — |
| `--rule` | `#2c3340` | borders, table rules (decorative) | — | — |
| `--text` | `#e7e9ec` | body text | `--bg` | 15.30:1 |
| `--text` | `#e7e9ec` | body text on cards | `--surface` | 14.05:1 |
| `--text` | `#e7e9ec` | text on inline code | `--surface-2` | 12.13:1 |
| `--muted` | `#a8b1bb` | secondary text | `--bg` | 8.57:1 |
| `--muted` | `#a8b1bb` | secondary text on cards | `--surface` | 7.87:1 |
| `--faint` | `#8a939e` | captions, attribution only | `--bg` | 5.98:1 |
| `--accent` | `#e8a75e` | brand amber, highlights | `--bg` | 8.97:1 |
| `--accent` | `#e8a75e` | accents on cards | `--surface` | 8.23:1 |
| `--accent-ink` | `#101318` | text on accent buttons | `--accent` | 8.97:1 |
| `--link` | `#7fb4e8` | links | `--bg` | 8.50:1 |
| `--link` | `#7fb4e8` | links on cards | `--surface` | 7.81:1 |
| `--ok` | `#8ec98f` | positive values, prompts | `--bg` | 9.66:1 |

Rule: `--faint` may not set body text. Use it for captions and table
attributions only. It passes AA (5.98:1) but not 7:1.
Rule: `--muted` on `--surface-2` measures 6.80:1. That passes AA. Do not
use it for long body text there; use `--text`.

### Light mode (`prefers-color-scheme: light`)

| Token | Hex | Role | On background | Ratio |
| --- | --- | --- | --- | --- |
| `--bg` | `#ffffff` | page background | — | — |
| `--surface` | `#f3f4f6` | cards, code blocks | — | — |
| `--surface-2` | `#e8eaee` | inline code, hover rows | — | — |
| `--rule` | `#d7dbe0` | borders (decorative) | — | — |
| `--text` | `#1b1f24` | body text | `--bg` | 16.56:1 |
| `--text` | `#1b1f24` | body text on cards | `--surface` | 15.05:1 |
| `--muted` | `#434c59` | secondary text | `--bg` | 8.69:1 |
| `--muted` | `#434c59` | secondary text on cards | `--surface` | 7.90:1 |
| `--faint` | `#5d6673` | captions only | `--bg` | 5.81:1 |
| `--accent` | `#a34d0e` | brand amber, dark variant | `--bg` | 5.79:1 |
| `--accent-ink` | `#ffffff` | text on accent buttons | `--accent` | 3.63:1 |
| `--link` | `#0e5aa3` | links | `--bg` | 6.98:1 |
| `--link` | `#0e5aa3` | links on cards | `--surface` | 6.34:1 |

Rule: light `--accent` passes AA for body size (5.79:1 ≥ 4.5:1). Use it for
short labels and links, not paragraphs. Light `--accent-ink` on `--accent`
(3.63:1) passes AA only for large text. Buttons must render at 17 px bold or
larger, or use `--text` on `--surface` instead.

## 3. Typography

No web fonts. jcode ships two JetBrains Mono woff2 files (~90 KB). rho's
claim is a small footprint, so the site ships zero font bytes. System fonts
render at first paint and cause no layout shift.

```css
--font-sans: ui-sans-serif, system-ui, -apple-system, "Segoe UI",
             Roboto, "Helvetica Neue", Arial, sans-serif;
--font-mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, Consolas,
             "Liberation Mono", "DejaVu Sans Mono", monospace;
```

Type scale (px / line-height). Base 17 px. Ratio ~1.25.

| Token | Size | Line height | Use |
| --- | --- | --- | --- |
| `--fs-caption` | 13 px | 1.5 | table attributions, footnotes |
| `--fs-small` | 15 px | 1.6 | table cells, code, nav |
| `--fs-body` | 17 px | 1.65 | body prose |
| `--fs-h3` | 21 px | 1.4 | subsection headings |
| `--fs-h2` | 26 px | 1.3 | section headings |
| `--fs-h1` | 33 px | 1.2 | page heading |
| `--fs-display` | 42 px | 1.1 | hero only, ≥768 px viewports |

At <768 px, `--fs-display` drops to 33 px and `--fs-h1` to 28 px.
Prose measure: max 68 ch. Page shell: max 880 px, padding 24 px.
Headings: weight 700, letter-spacing −0.02em. Body: weight 400.

## 4. Spacing, radii, shadows

Spacing scale (px): `4, 8, 12, 16, 24, 32, 48, 64, 96`. No other values.
Section gap: 64 (desktop), 48 (mobile). Paragraph gap: 16.

| Radius token | Value | Use |
| --- | --- | --- |
| `--r-0` | 0 | tables |
| `--r-s` | 6 px | buttons, inline code, badges |
| `--r-m` | 10 px | code blocks, cards |

Shadows: none in dark mode. Light mode may use
`0 1px 3px rgba(16,19,24,0.08)` on cards. No other shadow.
No gradients anywhere. No blur blobs. No decorative images.

## 5. Code blocks

- Background `--surface`, border `1px solid --rule`, radius `--r-m`.
- Padding 16 px. Font `--font-mono` at `--fs-small`.
- Shell prompt marker `$` in `--ok`; command text in `--text`.
- Inline code: `--surface-2` background, radius `--r-s`, padding 2px 6px.
- A copy affordance is optional. If present it is a real `<button>` with a
  visible focus ring. No clipboard icon without a label.

## 6. Tables

Tables carry the evidence. They get the most care.

- Full-width, `border-collapse: collapse`, radius 0.
- Header row: `--fs-caption`, uppercase, letter-spacing 0.06em, `--muted`,
  bottom border `2px solid --rule`.
- Body rows: `--fs-small`, bottom border `1px solid --rule`.
- Numeric columns: right-aligned, `--font-mono`, `font-variant-numeric:
  tabular-nums`.
- The rho row in any comparison table is highlighted with a
  `3px solid --accent` left border, not a background flood.
- Every table has a `<caption>` or an attribution line in `--faint` at
  `--fs-caption` directly under it, with source and sample date.
- Unmeasured rho values render as `to be measured` in `--muted` italic,
  never as a number.
- At <480 px wide tables scroll horizontally inside a wrapper with
  `overflow-x: auto`; they do not reflow into cards.

## 7. Interactive states

| State | Treatment |
| --- | --- |
| link default | `--link`, underline, `text-underline-offset: 3px` |
| link hover | `--text` in dark, `--accent` in light |
| button default | `--accent` bg, `--accent-ink` text, radius `--r-s`, padding 12px 24px, weight 700 |
| button hover | translate none; brightness 108%; underline none |
| secondary button | transparent bg, `1px solid --rule`, `--text` |
| focus (all) | `outline: 2px solid --accent; outline-offset: 2px` |
| disabled | `--faint` text, `--surface` bg, no pointer events |

Touch targets: minimum 44×44 px on interactive elements at mobile widths.
`prefers-reduced-motion: reduce` disables all transitions. Transitions,
where used, are `120ms ease` on color only. Never `transition: all`.

## 8. Accessibility floor

- Landmarks: one `<header>`, one `<main>`, one `<footer>`, `<nav>` labelled.
- One `<h1>` per page. No skipped heading levels.
- Skip link as first focusable element.
- `<html lang="en">`, `meta viewport` without `user-scalable=no`.
- Every ratio in section 2 is the shipped value. If a new pairing appears,
  compute its ratio before merge and record it here.
