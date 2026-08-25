# D-the-record-id-is-visible-on-the-command-line — a fork needs an id a user can see

**Question:** the owner asked to fork from any turn and any assistant message. A fork takes a
record id. Nothing shows a record id. So how does a user fork?

## The defect this fixes

A first draft named `/tree` and `/fork` in the command surface. Both were specified nowhere
and tested nowhere. Neither is in the terminal's command list today.

That is `D-dead-surface-is-a-defect-class`, word for word. It is also the `/guide` defect
again, where the first frame advertised a command that answered an error.

A product review put it plainly. The one thing the owner asked for was the one thing with no
usable path.

## The decision

The record id becomes visible on the command line, in this lane.

- `rho sessions show <id-prefix>` prints one record per line, with its id in the first column.
- The listing fits 80 columns, and a long text is cut rather than wrapped.
- A tool result row shows the tool and a byte count, never the content.
- `rho sessions list` gets six fixed columns, and they also fit 80 columns.

So the fork is two commands a user can type:

```sh
rho sessions show 20260825-09
rho sessions fork 20260825-09 --at r5
```

**`/tree` and `/fork` leave this lane.** The record navigator gets its own lane, its own spec,
and its own tests. Until then, nothing advertises it.

## Rules that hold

- One test reads a record id from the real `show` output, and then forks with it. So the test
  fails if the id is not printed. The owner's ask is pinned by that one test.
- `show` sends nothing to a model and starts no session. Looking costs nothing.
- `show` answers the other complaint too. A user could not look at a session without inventing
  a prompt, because `run` needs one.
- A printed surface is part of the contract. Its columns are specified, not left to the
  implementation.

## Rules out

**Naming a command that is not built.** The lane either specifies a surface and tests it, or
it says nothing about it.

**Making `show` a debug dump.** It is a user-facing view, so it has fixed columns and a width.

**Printing a tool result body.** A result can hold a secret, and redaction covers only a
secret-named argument. See `D-recording-is-on-by-default`.

**Waiting for the terminal navigator.** That would leave the owner's headline ask undelivered
for a whole lane, with no fallback.

## Cost

Two printed views with fixed columns, and fourteen tests. No change to the record format.
