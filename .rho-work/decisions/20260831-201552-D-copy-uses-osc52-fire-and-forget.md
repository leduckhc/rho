# D-copy-uses-osc52-fire-and-forget

Status: accepted
Date: 20260831-201552
Spec: SPEC-select-text-and-find-the-bottom

## The question

How does rho put selected text on the user's clipboard? What does the user see, given that
rho cannot confirm the write?

## The decision

rho copies with OSC 52. The frame is `\x1b]52;c;<base64>\x07`. The payload is the selected
text, base64-encoded. rho caps the copy at `COPY_BYTE_CAP`, which is 100 KB.

rho shows `sent NN bytes to the clipboard` on success. The word is "sent", not "copied",
because OSC 52 gives no reply. rho shows `selection too large to copy · limit 100 KB` when
the cap trips. rho does nothing when nothing is selected.

## Why

OSC 52 needs no new runtime dependency beyond base64, and it works over ssh. The project
already knows the sequence. See `crates/rho-tui/src/sanitize.rs:33`.

The text is the row's already-sanitised content. So the payload holds no escape sequence. A
malicious tool output cannot inject a control code into the clipboard write.

rho cannot detect a terminal that disables OSC 52, because the sequence returns nothing. An
honest word avoids a false claim. The external-editor path is the fallback for a terminal
that refuses OSC 52.

## What this rules out

- A native clipboard crate with an OS dependency.
- A "copied" message that claims a success rho cannot confirm.
- Copying unsanitised text.
- An unbounded copy.

## The risk

A user on a terminal without OSC 52 gets no clipboard write and no error. The `/help` copy
documents the limit and names the editor fallback.
