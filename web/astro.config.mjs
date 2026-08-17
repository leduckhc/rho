// @ts-check
import { defineConfig } from 'astro/config';

// https://astro.build/config
export default defineConfig({
  site: 'https://getrho.dev',

  // Keep the source whitespace. Astro's HTML compressor removes the newline
  // between a text run and an inline element, so `except\n<code>rho-core</code>`
  // renders as `exceptrho-core`. That defect is easy to write and hard to see.
  // The extra bytes are a few hundred before gzip. Correct text is worth more.
  compressHTML: false,
});
