# D-thinking-tag-case-insensitive — the leading reasoning tag matches any letter case

Date: 20260819

## The question

rho strips a leading `<thinking>` or `<think>` tag from an assistant message, and it draws
the text inside as dimmed reasoning. See `SPEC-reasoning-across-providers` section 3 "Two".

A model does not always write the tag in lower case. Some write `<Thinking>`. So the
splitter must decide whether the tag name match is case sensitive.

## The decision

**The tag name match ignores case.** `<thinking>`, `<Thinking>`, and `<THINKING>` all open a
reasoning block. The same rule applies to the short name `<think>`, to the closing tags
`</thinking>` and `</think>`, and to the self-closing `<thinking/>`.

The text inside the tag keeps its original case. Only the tag name is lower-cased for the
comparison.

## The reason

The tag is the model's own output, and a model is not consistent about case. A case
sensitive match would miss `<Thinking>` and draw it as the answer, which is the exact bug
this work fixes. HTML and XML tag names are case insensitive, so a reader expects the same
here. The cost is one `to_ascii_lowercase` call on a short prefix, which is nothing.

## What this rules out

- **No case sensitive mode.** There is no setting to demand lower case. A model that varies
  its case would then produce reasoning on one turn and a leaked tag on the next.
- **The tag name only.** The content case is never touched, so a reasoning body that names
  a proper noun stays readable.
