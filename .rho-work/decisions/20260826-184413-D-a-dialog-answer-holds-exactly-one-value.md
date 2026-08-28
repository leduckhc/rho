# D-a-dialog-answer-holds-exactly-one-value — no untagged dialog reply

Date: 20260826. Reference: `D-a-dialog-answer-holds-exactly-one-value`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 3.4.

## The question

The draft spec wrote the dialog reply as an untagged enum:

```rust
#[serde(untagged)]
pub enum DialogValue {
    Value { value: String },
    Confirmed { confirmed: bool },
    Cancelled { cancelled: bool },
}
```

Does an untagged reader read a dialog answer safely?

## What the compile showed

No. An untagged reader returns the **first** matching variant and reports nothing. This
project already shipped that defect once, in the session file format. See
`D-two-variants-cannot-share-a-serde-tag`.

Here the failure is worse than a wrong read. A client that sends
`{"confirmed":false,"cancelled":true}` gets `Confirmed(false)`. A client that sends
`{"value":"yes","cancelled":true}` gets `Value("yes")`. A dialog answer decides whether a
tool runs, so a guess here is a security decision made by field order.

A scratch crate outside the repository proved both the defect and the fix, at
`/tmp/rho-jsonl-contract-scratch`.

## The decision

`DialogAnswer` is a Rust enum with a private wire struct behind
`#[serde(try_from = "WireAnswer", into = "WireAnswer")]`. The wire struct holds three
optional keys and carries `deny_unknown_fields`. `TryFrom` counts the keys that are set. It
accepts exactly one, and it refuses zero and refuses two or more.

The refusal is loud. It arrives as a reply with `success: false` and
`error: "invalid_argument"`.

The wire shape is unchanged from the draft: `{"value":"a"}`, `{"confirmed":true}`, or
`{"cancelled":true}`. Only the reader changed.

## Why not the alternatives

- **Add a `method` tag to the answer.** The client would then have to repeat which dialog
  kind it is answering, and a wrong tag would be a second failure mode. The dialog `id`
  already says which request this answers.
- **Take the first key that is set.** That is the guess this decision removes.

## What it rules out

- No `#[serde(untagged)]` anywhere in `rho-jsonl`, except on `Reply`, where the two arms are
  separated by a one-value type and cannot both match. See the `True` and `False` types.
- No dialog answer that carries two answers.
