# A merged value error names the key and the value, and never a file

Date: 20260821. Reference: `D-a-merged-value-error-names-no-file`.
Spec: `docs/specs/20260820-200901-SPEC-config-call-site.md`, the error taxonomy.

## The question

`ConfigError::Parse` carries a path. Four parsers run **after** the merge, where no path
exists, and each passed the literal string "the merged configuration" as the path. The run
then printed this:

```text
rho: cannot parse the config file the merged configuration: the reasoning-effort key value
"ludicrous" is not valid
```

That is not a sentence. Two live runs found it: first for `tui-reasoning`, then again for
`reasoning-effort`. A second sighting means the shape was wrong, not the wording.

## The decision

Add `ConfigError::Value { key: &'static str, value: String, message: String }`. It prints:

```text
rho: the reasoning-effort value "ludicrous" is not valid: unknown reasoning effort ...
```

`parse_sandbox`, `parse_approval`, `parse_reasoning`, and `parse_reasoning_effort` all use it.
`Parse` keeps its path, and it stays the error for a real file.

## Why not a smaller change

Rewording the message would leave the lie in the type. A path field that never holds a path
invites the next parser to pass another sentence as one. The type now says what is true: a
merged value has a key and a value, and no file.

## What it rules out

- No parser passes a sentence as a `PathBuf`.
- No new error for the same case. A bad merged value is `Value`, whatever the key.

## What moved

Four tests asserted the `Parse` variant for a bad `sandbox` or `approval` value. Each keeps
its rule, which is that the run stops and the message names the key and the value. Only the
variant moved, and each test says so in a comment. The behaviour is unchanged, and every
value still fails closed.
