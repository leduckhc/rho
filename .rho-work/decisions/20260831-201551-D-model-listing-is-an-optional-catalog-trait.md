# D-model-listing-is-an-optional-catalog-trait

Date: 20260831-201551

## The question

How does rho let a provider list its models, when the three providers differ in kind?
Bedrock has `ListFoundationModels`. OpenRouter has an HTTP models endpoint. Azure names
deployments the account owner chose, not models.

## The decision

Listing is optional, and the type system carries that fact.

- A provider that can list implements a new `ModelCatalog` trait in `rho-core`.
- `Provider` gains one required method, `fn catalog(&self) -> Option<&dyn ModelCatalog>`.
- A provider that can list returns `Some(self)`. A provider that cannot returns `None`.
- `list_models` returns `Result<Vec<ModelDescriptor>, ProviderError>`. An empty `Ok(vec)`
  means "listed nothing". `None` from `catalog()` means "cannot list". They are distinct.

`catalog()` has no default body. Each provider states its answer, the way `is_leaf_record`
forces a record to state its class.

## What this rules out

- A `list_models` method on `Provider` itself. That forces Azure to lie or to error.
- A default body on `catalog()`. A default lets a new provider stay silent, and silence is
  the fail-open trap `AGENTS.md` step 8 names.
- A capability flag on the descriptor. See `D-a-model-descriptor-carries-no-capability-claim`.

## Why

Azure cannot list models. A single mandatory list method would force it to fake an answer.
An optional catalogue, expressed as `Option<&dyn ModelCatalog>`, lets Azure return `None`
without a lie, and lets a caller tell the two empty cases apart.
