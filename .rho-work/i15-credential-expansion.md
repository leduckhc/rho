# Expansion — I15, the providers resolve a credential through `Config`

Written 20260821-131638 UTC, before any file edit. Branch `feat/reasoning-across-providers`.
Spec: `docs/specs/20260820-200901-SPEC-config-call-site.md`. Branch record:
`.rho-work/reasoning-task.md`, which says "Next step: I15".

Ids in this file are local to this task. Read `S1` here as `I15-S1`, so it never clashes
with the branch-level ids in `.rho-work/reasoning-task.md`.

## TASK

Make every provider get its credential from the loaded config, so an absent key is an error
and the project trust gate finally runs.

## STATED

Each line names where it comes from: your reply, the reviewed spec, or real output.

- **S1.** "Wire the whole config now". *Your `ask_user` reply, S4 of the branch record.*
  The credential half is the part still unwired.
- **S2.** "No credential read through `std::env::var`. `provider.rs` stops using
  `unwrap_or_default()`, which turned an absent key into an empty string."
  *SPEC-config-call-site, section 2, "What the contract forbids".*
- **S3.** An absent credential is a `ConfigError`, "never an empty string".
  *Same spec, the error taxonomy table.*
- **S4.** The spec names the test `an_absent_credential_is_an_error_not_an_empty_key`.
  *Same spec, section 4.*
- **S5.** "Take the probe's mitigation (recommended)". *Your reply, S6 of the branch record.*
  The mitigation gates a command credential from an untrusted project file.
- **S6.** "Run 7 of the verification passed `--trust-project` and the command still did not
  run." *`docs/verification/config-call-site.md`, and the branch record.*
- **S7.** "The extension point: a new provider credential needs no edit to shared code,
  because `credentials` is a map from a provider name to a `CredentialSource`." *Same spec.*

## INFERRED

Mine, not yours. Each line carries its reason, so a wrong one is visible without code.

- **I1.** `build_provider` takes `&Config` and an `&dyn EnvLookup` — because it takes only a
  name today, so it cannot reach a resolved credential.
- **I2.** The credential name is the provider name, `openrouter` and `azure` — because S7
  says the map is keyed by a provider name.
- **I3.** Bedrock resolves no credential — because the AWS SDK reads its own chain, and
  `provider.rs` reads only the region for it.
- **I4.** All five `unwrap_or_default()` sites move together, at `provider.rs` lines 139,
  169, 192, 199, and 206 — because a site left behind keeps the defect.
- **I5.** The missing-credential error still names the concrete fix — because "no credential
  source is defined by that name" does not tell a user what to set.
- **I6.** A `RefusedProjectCommand` fails at resolve and exits non-zero, naming
  `--trust-project` — because a dropped value sends an empty key and reads as a 401.
- **I7.** A helper that is absent, not executable, or slow fails with a named
  `ConfigError::Credential` — because the 30-second timeout and the allowlist already exist,
  and a silent empty key becomes a 401.
- **I8.** No `Secret` reaches a log, an error, or a panic message — because `Secret` has no
  `Display`, and `D-one-redaction-home` holds.
- **I9.** A second provider build in one process resolves again — because the resolve happens
  at build time, not at load time, so a command helper runs twice. The second run must give
  the same answer, or fail the same way.
- **I10.** Step 11 re-drives run 7 on live Bedrock and on one keyed provider — because no
  fixture catches a request-side defect, and the gate has never run for real.
- **I11.** `README.md` line 66 and `docs/benchmarks.md` line 241 state the new precedence —
  because a user reads the doc, not the merge.
- **I12.** The malformed refusal sentence is fixed here: "cannot parse the config file the
  merged configuration" — because `AGENTS.md` forbids leaving a verified bug unfixed.

Coverage of the four usual sources: the failure path is I5 to I7, the second run is I9, the
wiring is I1 to I4 and I10, and the user-visible output is I5, I6, and I11.

## UNKNOWN

- **U1. The non-secret provider settings.** The AWS region, the Azure endpoint, and the Azure
  deployment are not secrets, and `resolve_credential` answers a `Secret`, which has no
  `Display`. Pick one: **(a)** leave those three on `std::env::var`, and move only the two
  real keys; **(b)** add three typed config keys, which changes the `Config` contract;
  **(c)** route them through the credentials map, which weakens `Secret`.
  *Default if you stay quiet: (a). It is the smallest change, and the spec forbids only a
  credential read, so a region read still obeys it. The cost is that a config file cannot
  yet set a region.*

- **U2. An absent `credentials` entry.** No user has a `credentials` table today. Pick one:
  **(a)** an absent entry falls back to the provider's documented variable, such as
  `OPENROUTER_API_KEY`; **(b)** an explicit entry becomes required.
  *Default if you stay quiet: (a). Option (b) breaks every existing user and two doc lines.*

- **U3. Where the fallback of U2(a) lives.** Pick one: **(a)** `rho-config` seeds a default
  `CredentialSource::Env` per known provider, so one rule holds for every caller;
  **(b)** `provider.rs` falls back after a `Credential` error, which puts provider names in
  the CLI.
  *Default if you stay quiet: (a). It keeps provider knowledge out of the call site, and it
  keeps the gate on the one path that resolves.*

## The U1 question was cancelled

Asked once, at 20260821-131638 UTC, and cancelled. A cancellation is not an answer, so the
stated default stands: **U1(a)**, leave the region, the endpoint, and the deployment on
`std::env::var`. U2(a) and U3(a) stand too. Work starts on those three defaults, and each one
is named in the commit, so a wrong default is cheap to reverse.

## Out of scope

- Reasoning R1 to R5, R9, and R10. A grep proves `ReasoningTrace`, `ReasoningReplay`, and
  `ReasoningWire` exist nowhere in `crates/`, so the branch's headline defect is still open.
- U2 and U4 of the branch record: the project `approval` and `sandbox` keys, and the stderr
  notices.
