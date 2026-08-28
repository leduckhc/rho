# D-a-tool-list-accepts-a-yaml-sequence — both spellings of a tool list are right

**Question:** the documented tool list is `tools: read, list`. A definition author wrote
`tools: [read, list]`, which is the ordinary YAML way to write a list. Is the sequence form
an error to report, or a form to accept?

## The decision

rho accepts both forms.

```yaml
tools: read, list
tools: [read, list]
tools:
  - read
  - list
```

The loader turns each form into the same token list, then applies the keyword rules of
D-a-tool-keyword-stands-alone. So `tools: [all]` inherits, and `tools: [all, read]` keeps
`read` and warns. One meaning, two spellings.

## Why accept it, rather than teach it

The file is YAML. A YAML sequence is the form a reader expects for a list, and every other
harness that reads a tool list accepts it. A refusal here would be rho being right about its
own document and wrong about the format. The comma form stays, because it is shorter and it
is already written in every example and in the guide.

## What a wrong value does

A value that is neither a string nor a sequence of strings is a rejection, not a warning. The
loader reports `BadToolsField` with the type it found, and the file does not load. The
alternative is to ignore the field, and an ignored `tools` field inherits the parent's whole
set. See D-a-rejected-definition-is-reported.

These cases reject:

| Value | Reason |
| --- | --- |
| `tools:` with nothing after it | The line states nothing. `none` states an empty set. |
| `tools: 5` | A number is not a tool name. |
| `tools: true` | A boolean is not a tool name. |
| `tools: [read, 5]` | One item is not a name. |
| `tools: {read: true}` | A map is not a list. |

The empty line needs care in the code. `Option<serde_yaml::Value>` reads a `tools:` line with
no value as `None`, which is the same value an absent field gives. So the field keeps the
difference with a custom `deserialize_with`, and an empty line arrives as `Some(Value::Null)`.
Without that, an empty line would inherit every parent tool in silence.

## Rules out

**A silent coercion of a number to a name.** `5` is never a tool, so the intersection would
drop it and the child would run with fewer tools than the file asked for. A rejection says so
at start-up instead.

**A nested sequence.** `tools: [[read]]` rejects. A tool name is a string.

**Accepting a sequence anywhere else.** `model`, `sandbox`, and `max_turns` stay scalars. No
author has written a sequence there, and a guess is how this defect started.

## Cost

One conversion function, and five tests.
