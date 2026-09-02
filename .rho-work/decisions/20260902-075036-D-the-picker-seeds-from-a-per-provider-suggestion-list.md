# D-the-picker-seeds-from-a-per-provider-suggestion-list

Date: 20260902-075036

## The question

A first-time user opens `/model` and sees one row: the current model. The starred file
is empty, and `Provider::list_models` is not built, per
`D-a-listing-failure-never-stops-a-session`. So the picker's fuzzy filter has nothing
to filter over, and a user with no favourites has no starting point.

Where do the rows come from?

## The decision

The frontend seeds the picker with a small, per-provider suggestion list. The list is
static, hard-coded in `rho-cli`, and passed to the App at build through
`App::with_suggested_models(Vec<String>)`. The picker draws:

1. The current model (position zero, always).
2. Every id in `~/.rho/starred-models.toml`, in file order.
3. Every id in the provider's suggestion list, in list order.

Rows are deduped by id, first occurrence wins. So the current stays at row zero, a
starred id keeps its starred flag when it also appears in the suggestion list, and a
suggestion for the current model is dropped.

The suggestion list is:

- **`openrouter`**: `anthropic/claude-haiku-4.5`, `anthropic/claude-sonnet-4.5`,
  `openai/gpt-5`, `openai/gpt-4o-mini`, `google/gemini-2.5-pro`.
- **`bedrock`**: `amazon.nova-micro-v1:0`, `amazon.nova-pro-v1:0`, `anthropic.claude-3-5-sonnet-20241022-v2:0`.
- **`anthropic`**: `claude-haiku-4-5`, `claude-sonnet-4-5`, `claude-opus-4-5`.
- **`azure`**: **none**. Azure names deployments, not models, and only the account
  owner knows the deployment names. See `docs/verification/models.md` and the reason
  the same table gives for skipping an Azure default.

A named provider entry (from `[[providers]]` in the config) with a known protocol is
matched by the protocol name, not by the id. So `xdent-claude` on the `anthropic`
protocol reads the `anthropic` suggestion list.

## What this rules out

- Reading the list from a file at runtime. A file has to migrate, a static list does
  not, and the list is measured in tens of ids at most.
- A `Provider::list_models` trait method. That is a separate feature that owes wire
  work per provider, plus caching, per `D-a-listing-failure-never-stops-a-session`.
  A hard-coded suggestion list has no wire work at all.
- A suggestion carrying a capability claim, a price, or a context length. See
  `D-a-model-descriptor-carries-no-capability-claim`. A suggestion is an id, one string.
- A ranked or scored suggestion list. The list is in the order a user with no context
  should try, and the fuzzy filter is what refines it.
- A first-open picker that lists every openrouter model. Openrouter carries several
  thousand ids, and a first user has no way to pick from a list that long.

## Why

A user typing `/model` with no starred file needs a starting point. Five well-known ids
per provider is the smallest useful nudge. Every one of them is documented in
`docs/verification/models.md` or in a shipped provider's tests, so nothing here is
guessed. A user who stars a suggestion promotes it; a user who wants a different id
types one, and the typed-fallback rule
`D-model-picker-allows-fuzzy-search-and-typed-fallback` covers that path.
