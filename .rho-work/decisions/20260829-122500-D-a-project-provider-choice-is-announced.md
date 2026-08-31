# D-a-project-provider-choice-is-announced — a notice, and not a gate

**Question:** the `credentials` table is refused in every form from an untrusted project file.
But `provider` is not gated, and a clone needs no `credentials` entry to benefit: it names the
provider whose key the user already has. Driven live:

```
$ printf 'provider = "openrouter"\nmodel = "none/none"\n' > .rho/config.toml   # untrusted
$ rho run "hi"                                                                # my own global key
rho: authentication failed: OpenRouter rejected the API key (status 401)       # my key travelled
```

Nothing said the project chose that.

**Decision: rho says so, once, and does not gate `provider`.**

```
rho: this project chose the openrouter provider, so your openrouter credential is in use.
Pass --provider to override it.
```

## Why a notice and not a gate

**It is not exfiltration.** An untrusted `base-url` is dropped, and that was confirmed live, so
the key still travels only to that provider's own endpoint. The clone cannot choose where the
key goes.

It is consent and cost. A repository decides which vendor bills the user, and which of their
keys is exercised. A reviewer reading the same code concluded the choice "grants nothing on its
own", which is true for secrecy and false for billing.

**Gating it would break a legitimate use.** A repository that works with one provider should be
able to say so. Requiring `--provider` in every checkout is the shape
`D-your-settings-are-a-floor` rules out for a limit, and the same reasoning holds here.

So the answer is the family of notice `base-url` already uses: rho obeys the value and says
what it obeyed.

## When it fires

Only when a project file set `provider` and **no stronger layer overrode it**. A notice that
blames the project for the user's own flag is worse than no notice.

The stronger-layer check decides exactly one case: the flag or `RHO_PROVIDER` names the **same**
provider the project did. When the values differ, the value comparison already settles it. A
mutation showed the first pair of tests never reached the check, because both used a different
provider, so deleting the check passed them. Two same-value tests now pin it, one per layer.

It fires under `--trust-project` too. That flag says the capabilities are safe to load. It does
not mean the user remembers which vendor the repository picked.

## Rules out

**Gating `provider` from an untrusted project file.** It breaks a legitimate per-repository
choice, and the endpoint is already protected.

**Refusing the run.** A 401 from the wrong vendor is recoverable, and a refusal would stop
honest work.

**Reusing `dropped_keys`.** That field's notice says "pass `--trust-project` to use them", and
nothing was dropped here. rho obeyed the value.

**A general `project_keys` list.** Only `provider` earns a notice today. A second key that
earns one is a second field, which is a smaller promise than a list every caller must filter.
