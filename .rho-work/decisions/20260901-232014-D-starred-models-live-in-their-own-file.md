# D-starred-models-live-in-their-own-file

Date: 20260901-232014

## The question

The user stars a model in the picker. The list must survive a restart. Where does rho keep
it, and who owns the write?

## The decision

The starred list lives at `~/.rho/starred-models.toml`. It is a **separate** file from the
main config. Format:

```toml
starred = [
  "anthropic/claude-sonnet-4-6",
  "amazon.nova-micro-v1:0",
]
```

- Values are provider-scoped model ids, exactly as `--model` accepts them. rho does not
  parse or split them.
- The TUI owns every write. No other code path writes this file.
- Reads happen once at startup and once on every open of the model picker (so an external
  edit takes effect on the next open).
- A missing file means an empty list. Not an error.
- A parse error is one notice to the user; the picker still opens with the current model
  only. rho never overwrites a file it could not read.
- The file is on the same disk as the session root, so `fs::rename` from a temp file is
  atomic. Every write goes through a temp-file-plus-rename dance.

## What this rules out

- A field on `rho_config::Config`. The merge, the profile depth, the untrusted-project
  filter, and the credential resolver have nothing to say about a favourite. A new field
  there would leak the trust rules onto a UI preference.
- A field on the session file. A star belongs to the user across sessions.
- A write inside the config file. rho would then rewrite a hand-authored comment.
- A star that carries a provider or an effort. The id alone is what identifies a model in
  the wire request. See D-a-model-descriptor-carries-no-capability-claim.

## Why

The starred list is a UI preference. It does not gate a security decision. Keeping it in
its own file keeps `rho_config` free of a UI concern, keeps the file writable without a
merge, and keeps a first-time user editable copy of the main config untouched. The
`~/.rho/` root is already the one place every user-scoped file lives, so the star file
sits beside `config.toml`, and a user finds them together.
