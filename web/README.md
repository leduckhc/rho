# getrho.dev

The static website for rho. Built with [Astro](https://astro.build).
The site ships no web fonts and no client-side JavaScript on the
critical path.

## Develop

```sh
cd web
npm ci
npm run dev
```

The dev server prints a local URL. Open it in a browser.

## Build

```sh
cd web
npm ci
npm run build
```

The build writes static files to `web/dist/`.

## Design system

The design tokens are owned by [`web/design/DESIGN.md`](design/DESIGN.md).
Do not change a colour, a size, or a spacing value in
`src/styles/site.css` without changing `DESIGN.md` first.

- `web/design/DESIGN.md` — tokens, type scale, table and code-block rules.
- `web/design/RECOMMENDATION.md` — the page outline and the chosen direction.
- `web/design/mockup-c-parts.html` — the frozen reference mockup (D-004).
- `web/src/styles/site.css` — the one stylesheet. It copies the tokens.

## Content rules

- No rho performance number appears on the site (decision D-003).
  Unmeasured rho values render as `to be measured` and link to
  `docs/benchmarks.md`.
- Every number about another harness carries its source and sample date.

## Deploy — Cloudflare Pages

Do not deploy by hand. Connect the repository to Cloudflare Pages with
these settings:

| Setting | Value |
| --- | --- |
| Build command | `npm run build` |
| Build output directory | `dist` |
| Root directory | `web` |
| Node version | `22` (set `NODE_VERSION=22`) |

The production domain is `https://getrho.dev`.
