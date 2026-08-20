#!/usr/bin/env node
// Check a TUI design mock against the invariants the renderer must keep.
//
// A mock is a contract, not a picture. `frames.rs` pins the renderer to the shape a mock
// states, so a mock that lies costs a defect. This script fails on four lies:
//
//   1. A line that is not exactly the frame width. A drifting column is the defect the
//      frame fixtures exist to catch, so a mock may not hide one.
//   2. A band that is not its stated height. A mock must not hide a row the code has.
//   3. A key the mock names that no binding row answers. That is
//      `D-a-panel-nobody-can-open`. `ctrl-t` shipped in a mock footer this way.
//   4. A composer over its row cap: 10 draft rows, 12 with both rules.
//
// Usage: node docs/design/check-mock.js docs/design/tui-mock.html [more.html ...]

const fs = require("fs");
const path = require("path");

const REPO = path.resolve(__dirname, "..", "..");
const BINDINGS = path.join(REPO, "crates", "rho-tui", "src", "bindings.rs");
const RENDER = path.join(REPO, "crates", "rho-tui", "src", "render.rs");
const DRAFT_ROWS_CAP = 10;
const COMPOSER_ROWS_CAP = DRAFT_ROWS_CAP + 2;

/** The real band height, read from the renderer. The mock must state this number, or a
 *  mock could hide a row the code has by lowering its own constant. */
function realBandRows() {
  const src = fs.readFileSync(RENDER, "utf8");
  const m = src.match(/pub const BAND_ROWS:\s*u16\s*=\s*(\d+)/);
  if (!m) throw new Error("BAND_ROWS not found in render.rs");
  return Number(m[1]);
}

/** Every individual key token the real binding table names. */
function realKeyTokens() {
  const src = fs.readFileSync(BINDINGS, "utf8");
  const tokens = new Set();
  for (const m of src.matchAll(/keys:\s*"([^"]+)"/g)) {
    // A binding may be a chord, for example `ctrl-x ctrl-e`. Both halves are real keys.
    for (const token of m[1].split(/[\s,]+/)) if (token) tokens.add(token);
  }
  return tokens;
}

/** Load a mock's frame logic without its DOM wiring. */
function loadMock(file) {
  const html = fs.readFileSync(file, "utf8");
  const script = html.match(/<script>([\s\S]*)<\/script>/);
  if (!script) throw new Error("no <script> block");
  let code = script[1];
  const wiring = code.search(/const nav|const foot|document\.|nav\.append/);
  if (wiring > 0) code = code.slice(0, wiring);
  const api = new Function(
    code + "; return { FRAMES, render, BAND_ROWS, composer };"
  )();
  if (!api.FRAMES || !api.render) throw new Error("no FRAMES or render()");
  return { api, html };
}

/** The visible text of one rendered line, with markup and entities resolved. */
function plain(line) {
  return line
    .replace(/<[^>]+>/g, "")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

function checkFile(file) {
  const findings = [];
  const notes = [];
  const { api, html } = loadMock(file);

  // ---- 1 and 2: every line is its width, and the band is its stated height. ----
  // The mock's own constant is checked against the renderer first, so a mock cannot pass
  // by lowering its band height to match a short frame.
  const real = realBandRows();
  const band_note = (text) => {
    if (PROPOSE) notes.push(text);
    else findings.push(text);
  };
  if (api.BAND_ROWS !== real) {
    band_note(`BAND_ROWS is ${api.BAND_ROWS}, and render.rs states ${real}`);
  }
  let frames = 0;
  for (const [name, frame] of Object.entries(api.FRAMES)) {
    frames++;
    const width = frame.w || 100;
    // An elastic variant may draw fewer rows, and it must declare how many. It may never
    // declare more than the renderer allows.
    const wanted = frame.bandRows ?? api.BAND_ROWS;
    if (wanted > real) {
      band_note(`${name}: declares ${wanted} band rows, and the cap is ${real}`);
    }
    let regions;
    try {
      regions = [...api.render(frame).matchAll(/<pre>([\s\S]*?)<\/pre>/g)].map(
        (m) => m[1]
      );
    } catch (error) {
      findings.push(`${name}: render() threw — ${error.message}`);
      continue;
    }
    regions.forEach((region, index) => {
      region.split("\n").forEach((line, row) => {
        const text = plain(line);
        const len = [...text].length;
        if (len !== width) {
          findings.push(
            `${name}: region ${index} row ${row + 1} is ${len} columns, not ${width} — ${JSON.stringify(text.slice(0, 48))}`
          );
        }
      });
    });
    const band = regions[regions.length - 1].split("\n").length;
    if (band !== wanted) {
      findings.push(`${name}: the band drew ${band} rows, and it states ${wanted}`);
    }
  }

  // ---- 3: every key the mock names answers a real binding. ----
  const realKeys = realKeyTokens();
  for (const line of html.split("\n")) {
    if (/NOT BUILT|not built/i.test(line)) continue; // an unbuilt key must say so
    for (const m of line.matchAll(/\b(ctrl-[a-z]|alt-[a-z]|alt\+enter|shift\+enter)\b/g)) {
      if (!realKeys.has(m[1])) {
        findings.push(
          `${m[1]} is named, and no binding row answers it — see D-a-panel-nobody-can-open`
        );
      }
    }
  }

  // ---- 4: the composer keeps its row cap. ----
  if (typeof api.composer === "function") {
    const draft = (n) => Array(n).fill("  draft");
    let capped = false;
    try {
      api.composer(draft(DRAFT_ROWS_CAP + 1));
    } catch {
      capped = true;
    }
    if (!capped) {
      findings.push(
        `composer() accepted ${DRAFT_ROWS_CAP + 1} draft rows, and the window is ${DRAFT_ROWS_CAP}`
      );
    }
    try {
      const rows = api.composer(draft(DRAFT_ROWS_CAP)).length;
      if (rows !== COMPOSER_ROWS_CAP) {
        findings.push(
          `composer() drew ${rows} rows at the cap, and it must draw ${COMPOSER_ROWS_CAP}`
        );
      }
    } catch (error) {
      findings.push(`composer() refused a full draft — ${error.message}`);
    }
  }

  // ---- 5: no row is hand-spaced. ----
  // Hand-counted spacing is how 38 right-aligned elements in this mock's first draft ended
  // 11 to 24 columns short of the width while the renderer put them flush. A row with a
  // right-aligned segment must use `{ j: }`, a two-column row `{ col: }`, and a selected
  // row `{ sel: }`. So no authored line may carry a run of three or more spaces.
  for (const [name, frame] of Object.entries(api.FRAMES)) {
    for (const region of ["scroll", "band"]) {
      for (const item of frame[region] ?? []) {
        const rows = typeof item === "string" ? [item] : (item.composer ?? []);
        for (const row of rows) {
          if (typeof row === "string" && /\S {3,}\S/.test(row)) {
            findings.push(
              `${name}/${region}: a hand-spaced row, so use { j: } or { col: } — ${JSON.stringify(row.replace(/«[a-z]+\|/g, "").replace(/»/g, "").slice(0, 44))}`
            );
          }
        }
      }
    }
  }

  return { findings, notes, frames };
}

const files = process.argv.slice(2).filter((a) => !a.startsWith("--"));
// A variant is a proposal, so it may argue for a different band height. `--propose`
// reports a band-height difference as a note instead of a finding. The baseline mock is
// always checked without it, because the baseline must state the renderer's real number.
const PROPOSE = process.argv.includes("--propose");
if (files.length === 0) {
  console.error("usage: node docs/design/check-mock.js [--propose] <mock.html> [...]");
  process.exit(2);
}

let total = 0;
for (const file of files) {
  let result;
  try {
    result = checkFile(file);
  } catch (error) {
    console.log(`${path.basename(file)}: UNREADABLE — ${error.message}`);
    total++;
    continue;
  }
  for (const note of result.notes ?? []) {
    console.log(`${path.basename(file)}: note · ${note}`);
  }
  for (const finding of result.findings) {
    console.log(`${path.basename(file)}: ${finding}`);
  }
  total += result.findings.length;
  console.log(
    `${path.basename(file)}: ${result.frames} frames, ${result.findings.length} findings`
  );
}
console.log(`VIOLATIONS ${total}`);
process.exit(total === 0 ? 0 : 1);
