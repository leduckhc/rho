# D-a-command-is-strict-and-an-event-is-loose

Date: 20260826. Reference: `D-a-command-is-strict-and-an-event-is-loose`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, sections 3.1 and 8.

## The question

AGENTS.md step 3 asks one question of every contract: what does an old reader do with a new
field? The JSONL protocol has two readers. rho reads a command. A client reads an event and a
reply. Do both readers follow the same rule?

## The decision

No. The two directions get opposite rules, and each direction says why.

**rho reads a command strictly.** `Command` carries `deny_unknown_fields`. A newer client
that sends a field an older rho does not know gets a loud reply:
`success: false`, `error: "parse_error"`, and a message naming the field.

A command is an instruction. An unknown field may be the part that limits the instruction. If
rho drops it and acts on the rest, rho does more than the client asked. So a command with an
unknown field is refused whole.

**A client reads an event loosely.** No event type and no reply type carries
`deny_unknown_fields`. A client must ignore an unknown field, and it must ignore an unknown
`type` value. That is the only compatibility guarantee the protocol makes.

An event is a report. A client that drops a field it does not know shows less than happened,
which is a display gap and not a wrong action.

## The consequence for a new command

A new command must be a **new `type` value**, never a new field on an existing type. Adding
a field is a breaking change under this rule, and the rule says so out loud.

An unknown `type` gets `error: "unknown_command"`. So a client can probe for a command and
read the answer, instead of guessing from a version number. The protocol carries no version
field, and this is why it needs none.

## Why not one rule for both

- **Strict in both directions.** A client would then break on every new event rho adds. rho
  could never add an event.
- **Loose in both directions.** rho would then obey half of a command it did not understand.
  A half-obeyed instruction with a `success: true` reply is the worst of the four cases.

## What it rules out

- No `deny_unknown_fields` on `Event`, `Reply`, `ReplyOk`, `ReplyErr`, or `DialogRequest`.
- No new field added to an existing `Command` variant as a way to extend the protocol.
- No protocol version field. The reply to an unknown command is the discovery mechanism.
