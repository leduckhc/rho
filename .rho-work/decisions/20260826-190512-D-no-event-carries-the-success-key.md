# D-no-event-carries-the-success-key — the routing rule needs a reserved field

Date: 20260826. Reference: `D-no-event-carries-the-success-key`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, sections 3.2 and 3.3.

## The question

Replies and events share one stream. The spec told a client how to tell them apart:

> Distinguish them by the top-level `"success"` field: replies carry it, events do not.

Does every event obey that rule?

## What the test showed

No. The draft spec, and the first implementation, both gave one event a `success`
field:

```json
{"type":"tool_end","id":"t1","success":true}
```

A client that routes on the presence of `success` reads that event as a reply. It then
looks for a `command` field, finds none, and either fails or drops the line. A tool that
finished would look like a malformed reply.

An invariant test found it, not a review. `an_event_has_a_type_and_no_success_field`
builds one of every `Event` variant and asserts two things about each: it carries `type`,
and it does not carry `success`. That is a test of the pairing rather than of one
example, which is what AGENTS.md step 12 asks for.

## The decision

`success` is a **reserved field name on the wire**. Only a reply may carry it. No event
may carry it, at any nesting level of its top-level object.

`Event::ToolEnd` names its field `ok`. It is the inverse of `ToolOutput::is_error`, and
the name says the same thing without taking the reserved word.

The routing rule is now stated from both sides, so a client needs only one of them:

- An event carries `type`, and never `success`.
- A reply carries `command` and `success`, and never `type`.

## Why not the alternatives

- **Change the routing rule to "look for `type`".** It works, but it leaves the trap in
  place for the next event somebody adds. A reserved name plus a test that enforces it
  removes the whole class.
- **Wrap every event in an envelope, such as `{"event": {...}}`.** It costs every client
  an extra level of nesting, and it breaks the pi-style flat shape the protocol keeps on
  purpose.

## What it rules out

- No event field named `success`, ever. The invariant test fails if one appears.
- No reply field named `type`.
- No routing rule that depends on a field being absent alone.
