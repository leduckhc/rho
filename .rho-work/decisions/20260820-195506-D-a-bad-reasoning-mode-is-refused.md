# D-a-bad-reasoning-mode-is-refused — every source errors on an unknown reasoning mode

Date: 20260820

## The question

rho takes the reasoning display mode from three places: the `--reasoning` flag, the
`RHO_TUI_REASONING` variable, and the `tui-reasoning` key in a config file. The three did
not agree about a value that is not a mode name, such as `loud`.

- The flag fell back to `summary` in silence. See `resolve_reasoning` in
  `crates/rho-cli/src/cli.rs`, which discarded the error with `.ok()`.
- The variable fell back the same way, through the same call.
- The config file returned a typed error. See `parse_reasoning` in
  `crates/rho-config/src/lib.rs`.

`SPEC-reasoning-across-providers` section 5 names the test
`an_unknown_reasoning_mode_is_refused` and says the mode "does not fall back in silence".
Two committed tests then asserted opposite things:
`a_bad_flag_value_falls_back_to_summary` in `rho-cli`, and
`a_bad_reasoning_value_fails_closed` in `rho-config`.

## The decision

**An unknown mode name is an error, at every source.** rho writes one line to stderr that
names the source, the bad value, and the four valid names, and it exits non-zero. It draws
nothing.

The owner ruled this on 20260820: "if the argument value is wrong, throw error".

`a_bad_flag_value_falls_back_to_summary` is deleted, and the doc comment that defended the
fallback goes with it. `a_bad_reasoning_value_fails_closed` stays, because it already
asserts this rule for the file layer.

## The reason

One meaning gets one behaviour. A user who writes `--reasoning loud` made a typing mistake,
and the mistake is cheap to fix once rho names it. A silent fallback hid the mistake and
drew a mode the user did not ask for, so the user learned nothing and the setting looked
broken.

The split was worse than either half. The same wrong word stopped rho from a file and passed
from a flag, so no user could predict the result.

The cost is stated: an exported `RHO_TUI_REASONING` with a stale value now stops the TUI
instead of drawing the default. The error names the variable and the valid names, so the
repair is one command.

## What this rules out

- **No warn-and-continue.** A warning on stderr scrolls out of sight in the TUI, so it is
  not a report.
- **No source-specific behaviour.** A flag, a variable, and a file key are refused the
  same way. The split above is the defect, not a design.
- **No new fallback mode.** `summary` stays the default only when no source sets a value.
- **Not a security claim.** A display mode is not a permission. This decision is about one
  predictable behaviour, and `D-plugin-does-not-classify-itself` still owns the fail-closed
  rules for policy.
