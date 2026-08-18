# D-provider-extension-verified-outside — The provider extension point is verified from outside the workspace


`README.md` and decision D-provider-contract-crate both claim that a third party can implement
`rho_core::Provider` and prove it conforms, without forking rho. That claim was
never tested. An untested claim about an extension point is a slogan.

**What the controller did.** Built a scratch crate at `/tmp/outsider`, outside the
rho workspace, depending on `rho-core` and, as a dev dependency,
`rho-provider-testkit`. Implemented a toy `Provider`, wrote one `ProviderHarness`
bridge, and called `run_all(&ToyHarness).await`.

**Result: the claim holds.** One call runs every conformance check. The suite then
correctly **rejected** a deliberately broken provider that omitted its first event:

```
the first event must be MessageStart, but it was TextStart { index: 0 }
```

So the suite discriminates, and its message names the violation.

**One real finding, about discoverability rather than design.** Writing the outside
crate needed three guesses at the API, and all three were wrong. The trait method is
`id`, not `name`. The usage event is `StreamEvent::Usage(Usage)`, not a struct
variant. The `HarnessRun.guard` field needs a `Box<dyn Any + Send>` even when a
provider has nothing to guard.

An outsider hits every one of those. So the crate now carries a `README.md` with a
working bridge, a table saying what each `Script` must produce, and the list of
checks. The types were always right. Only the front door was missing.
