# Task — reasoning across providers, and the config call site it grew into

Working notes for branch `feat/reasoning-across-providers`. Specs:
`docs/specs/20260819-134615-SPEC-reasoning-across-providers.md` and
`docs/specs/20260820-200901-SPEC-config-call-site.md`.

## TASK

Make rho show a model's reasoning correctly on every provider. Wire the config that
feature needs, and keep a cloned repository from running commands on startup.

The second sentence was not in the original task. It grew from one ruling. Section
`UNKNOWN` asks whether that growth is still wanted.

## STATED

Each item is a quote. Four came from `ask_user` replies, so the quote is the option title
you picked. Two questions were **cancelled**, and that is recorded here as well, because a
cancellation is not an answer.

- **S1.** "what's this branch about? is it finished?"
- **S2.** "write the task we have at hand" and "write the list of requirements we have"
- **S3.** "what do you mean by refuse? if the argument value is wrong, throw error"
  → This is the R6 ruling, in your own typed words. The `ask_user` question about R6 was
  cancelled, and you answered in prose instead.
- **S4.** "Wire the whole config now"
  → This overruled `D-the-layered-config-has-no-caller`, which had deferred the wiring.
- **S5.** "Trust the project file fully, as the spec reads today"
- **S6.** "Take the probe's mitigation (recommended)"
  → This narrowed S5 after a live probe proved command execution at startup.
- **S7.** "continue pi session 01a020b6-173e-7c66-8086-9d2342040dba"
- **S8.** "turn the task and requirements list into Stated / Inferred / Unknown"

Two cancellations, recorded because they shaped the work:

- **S9.** The R6 question was cancelled. S3 replaced it.
- **S10.** The trust carve-out question was cancelled, so S5 stood until S6 replaced it.

## INFERRED

What I filled in, and the evidence for each. Anything here is mine, not yours, so it is
the first place to look for a wrong assumption.

### Already built on these

- **I1.** Refuse means a typed error and a non-zero exit, at **every** source: the flag,
  the variable, and the file. S3 said "throw error" about an argument. I applied it to all
  three, because one meaning with two behaviours is the defect R6 already was.
  *Shipped in `2f807c4`.*
- **I2.** `--reasoning loud` and `RHO_TUI_REASONING=loud` both fail. The committed test
  `a_bad_flag_value_falls_back_to_summary` is deleted, because S3 asserts the opposite of it.
  *Shipped in `2f807c4`.*
- **I3.** Config discovery reads `XDG_CONFIG_HOME`, then `HOME`. No path literal, no `dirs`
  dependency, and no `HOME` lookup existed anywhere, so S4 needs discovery invented.
- **I4.** An exported-but-empty `HOME` counts as unset. Joining from `""` would name
  `/rho/config.toml` at the filesystem root, which no user means.
- **I5.** Discovery is pure, and it never tests the filesystem. `Config::read_file` already
  answers `Ok(None)` for an absent file, so a second existence check would be two answers.
- **I6.** `Sources` fields become `pub(crate)`, with one constructor and one method per
  source. They were `pub`, so `Sources { ..Default::default() }` walked around any rule
  written as prose. This cost a rewrite of 20 construction sites across 7 files.
- **I7.** An untrusted project command becomes `RefusedProjectCommand`, not a dropped value.
  A drop would hand the provider an empty key and a 401, which reads as a broken account.
- **I8.** The refusal names `--trust-project` in its message, so the fix is discoverable.
- **I9.** A refusal fires only when the value still matches what the untrusted project file
  held. A profile may replace it, and that later value is not the one the gate refused.
- **I10.** A global file is never gated. A home directory is not a clone.
- **I11.** The gate covers a command credential, `skill-paths`, and `mcp-config`. S6 named
  the probe's mitigation, and those are the three keys the probe named.

### Chosen but not yet built

- **I12.** `--profile` must exist. Layer 4 selects a profile, and `Cli` has no such flag,
  so profiles are unreachable today.
- **I13.** `read_only`, `mouse`, and `no_skills` become `Option<bool>`, and `sandbox`
  becomes `Option<SandboxArg>`. `sandbox` carries a clap default today, so layer 6 would
  always beat a file asking for `sandbox = "strict"`. A review named that a fail-open
  inside the merge built to prevent one.
- **I14.** The clap `env` attribute is dropped from `--provider`, `--model`, and `--log`.
  `SPEC-config` section 2 already forbids a second precedence.
- **I15.** Credentials resolve through `Config::resolve_credential`. `provider.rs` uses
  `std::env::var(...).unwrap_or_default()` in five places, so an absent key becomes an
  empty string and a 401.
- **I16.** The load happens once per process, and every later reader takes `&Config`.

## UNKNOWN

I cannot decide these. Each needs one choice from you.

- **U1. Priority. Does the config work continue, or does reasoning come first?**
  S4 has grown far past the reasoning feature. R1 to R5, R9, and R10 of the reasoning spec
  are all still open, and the reasoning defect you first reported is among them. Pick one:
  **(a)** finish the config call site, then return to reasoning; **(b)** park the config
  work now that the security gate is in, and go do reasoning.
  *This is the same question the previous session ended on, and it is still unanswered.*

- **U2. `approval` and `sandbox` from a project file.** S5 trusts them and S6 did not
  narrow them. The probe rated them High and Medium-High. A cloned repository can still
  set your approval mode and your sandbox mode. Pick one: **(a)** leave them trusted, as
  S5 says; **(b)** add them to the gate of S6.

- **U3. `--read-only` against an `approval` key.** Both exist, and only the flag is read
  today. One must win, and no ruling names which.

- **U4. The stderr announcements.** Rules 4, 7, and 8 say rho reports a loaded file, an
  absent global path, and a dropped capability. No named test asserts any of that text, and
  `Config` has no field to carry it. Pick one: **(a)** add a notices field to `Config`, which
  is a contract change; **(b)** have the CLI recompute what to report; **(c)** drop the rules.

- **U5. Where the reasoning tag rule and `ask_to_enable` meet.** Both fix the same reported
  defect from opposite ends. The spec keeps both, and no test pins what happens when a model
  returns a structured block **and** writes a tag in the same turn. The spec's prose says the
  tag is left alone in that case. No test proves it.

## State of the tree

HEAD is `01cd81c`. Two commits landed in this branch beyond the rescue commit.

| Commit | What it did |
| --- | --- |
| `e08b190` | rescued 1668 unreviewed lines, and vouched for none of them |
| `2f807c4` | R6: an unknown reasoning mode is refused at every source |
| `01cd81c` | the config call-site contract, reviewed before any code |

Uncommitted, and gate-green at **874 tests**: config discovery and the project trust gate.
That is `ConfigPaths::discover`, `ProjectTrust`, the `Sources` builder, and
`CredentialSource::RefusedProjectCommand`, with 17 new tests in two files.

**Step 7 found two of my own tests worthless, and that is the day's most useful result.**
Three deliberate breaks caught only one failure at first.

| Break | Caught at first? |
| --- | --- |
| `XDG_CONFIG_HOME` precedence inverted | yes |
| the two path slots swapped in `from_paths` | **no** |
| the empty-value check deleted | **no** |

The swap hid because each file set a different key, so both values still arrived. Both files
now set the same key, so the winner reveals the order. The empty case hid because the test
set no variable at all, so it never reached the check. Two tests were added for it.

Four more breaks were then run against the trust gate, and each one failed a test. One
break downgraded the refusal to an empty literal, which is the exact silent drop
`AGENTS.md` names. `an_untrusted_project_command_never_runs_the_command` proves the gate by
running a real command and checking that the marker file is absent.

## Next step

Blocked on U1.
