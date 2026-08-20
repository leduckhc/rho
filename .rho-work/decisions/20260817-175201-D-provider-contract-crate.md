# D-provider-contract-crate — The provider contract suite is a real crate, not a private test file


`workflow.yaml` put the shared provider contract suite in
`crates/rho-core/tests/provider_contract.rs`. The controller changed this.

**Decision:** the suite lives in a new crate, `rho-provider-testkit`. It exports
reusable functions that assert the contract against any `Provider`
implementation. Each provider crate calls those functions from its own tests.

**Reason, and it is a product reason, not a convenience.** rho's thesis is that a
third party writes a provider without forking rho. A third party cannot run a
test file that is private to `rho-core`. A testkit crate lets any author prove
conformance with the same assertions we use. The extension point stops being a
claim and becomes a tool.

**Second reason:** it removes a file collision. Stage S4 must prove it did not
touch `crates/rho-core/tests/`, and a new file there would break that proof.

**Constraint:** `rho-provider-testkit` is a normal crate, not a dev-dependency
hack. It depends only on `rho-core`, `tokio`, `serde`, `serde_json`, `futures`,
and `async-trait`. It never depends on a concrete provider.
