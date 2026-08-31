# Reasoning tokens are optional, and absent never means zero

Date: 20260831. Reference: `D-reasoning-tokens-are-optional-never-zero`.
From `SPEC-usage-carries-reasoning`.

## The question

The user wants to see reasoning tokens in the terminal. `rho_core::Usage` has no such
field. A new field can be a `u64` with a default, or an `Option<u64>`. The two behave
differently when a provider omits the number, and when an old file is read.

## The decision

Add `reasoning_tokens: Option<u64>` to `rho_core::Usage`. Use
`#[serde(default, skip_serializing_if = "Option::is_none")]`. This mirrors `cost_usd`.

- `None` means the provider reported no count.
- `Some(0)` means the provider measured zero.
- The provider crate owns this distinction. It sets `None` when it reads no field.
- No shared code and no harness may substitute `Some(0)` for a missing number.

A `u64` with a default would read a missing key as `0`. An old session file, and a
provider that reports nothing, would both show `0` reasoning tokens. That is a claimed
measurement nobody made. `AGENTS.md` forbids an estimate. So the field is an `Option`.

An old rho reads a new `usage` record without error. `Usage` has no
`deny_unknown_fields`, so serde ignores the extra key. The old rho drops
`reasoning_tokens` on read, and `fork` erases it on disk. This is acceptable. The number
is derived provider data, re-produced on the next turn. It is never a boundary decision,
so a silent drop here is safe, unlike a dropped policy field.

## What this rules out

- No `u64` field. A default of `0` would lie.
- No count invented from text length or character count.
- No `Some(0)` written by shared code or by the testkit to stand for "unknown".
- No change to `Entry` to add `deny_unknown_fields`. That is a separate defect.
