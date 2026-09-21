# Prior art: how pi switches model and provider mid-session

The user asked rho to switch the provider while a session runs. They named pi as the
design they like. AGENTS.md step 1 says to read the prior art from real source, not from
memory. pi is installed on this machine, so every claim below is measured.

Source root: `/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent/`.

This file is evidence for `F-model-picker` and for the provider-switch spec. It states what
pi does. It does not decide what rho does.

## 1. The switch is atomic over the pair

`docs/rpc.md:245` gives the control message:

```json
{"type": "set_model", "provider": "anthropic", "modelId": "claude-sonnet-4-20250514"}
```

The provider and the model move together, in one operation. pi has no separate
`set_provider` call.

**What this means for rho.** Do not design two setters. A caller that sets a provider and
then a model leaves a window where the pair is invalid.

## 2. The persisted record already matches rho

`docs/session-format.md:218` shows the record pi appends:

```json
{"type":"model_change","provider":"openai","modelId":"gpt-4o"}
```

The writer API at `docs/session-format.md:409` is `appendModelChange(provider, modelId)`.

rho already writes `Record::ModelChange { provider: String, model: String }`. The shapes
agree, so a provider switch needs no new record type.

## 3. Model identity is the pair, written `provider/modelId`

`docs/extensions.md:1017` says pi resolves scoped models "with minimatch on
`provider/modelId` or a bare `modelId`".

So pi treats `provider/modelId` as the canonical name. A bare model id is a loose match.

**What this means for rho.** `~/.rho/starred-models.toml` stores bare ids today. Two
providers can expose the same id. A star is therefore ambiguous once rho lists more than
one provider. The file format needs a decision, and the decision must say what a new rho
does with an old file, and what an old rho does with a new file.

## 4. Per-model effort is keyed by the pair

`docs/settings.md:33` describes `modelThinkingLevels` as "Per-model startup thinking levels
keyed by `provider/modelId`".

rho keeps a preview effort on each picker row. That preview needs the same key, or two
providers with one shared model id will share one effort by accident.

## 5. pi does not list a model whose provider has no credential

`docs/models.md` says pi "treats models as requiring auth before they appear in `/model`".
It tells the user to keep a dummy key for a keyless local server, or to save one with
`/login`.

**What this means for rho.** This answers most of the cost question and part of the trust
question. A provider with no usable credential is never listed. rho then makes no network
call for it, and reads no key it does not have. rho should adopt this rule or reject it in
writing.

## 6. A provider may not support the current effort

pi carries `compat` flags per provider and per model. `docs/models.md` names
`supportsReasoningEffort` and `supportsDeveloperRole`. A server that cannot take an effort
level is a normal case, not an error.

rho already has a test named `model_switch_drops_reasoning_bound_to_old_model`. A provider
switch raises the same question again. The spec must say what happens to the effort when
the new provider cannot honour it.

## What rho should not copy without thought

pi stores providers in `~/.pi/agent/models.json`. rho already has a `[[providers]]` table
in its own config, with a credential name per entry. rho keeps its own shape here, because
its trust rules differ. A project-level rho config may not add a credential. See
`docs/specs/20260901-192553-SPEC-named-provider-profiles.md`.
