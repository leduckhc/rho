# D-a-provider-with-no-credential-is-not-listed

Date: 20260921-092638

## The question

The `/model` picker will list models from every configured provider. rho compiles several
built-in providers, such as `bedrock`, `azure` and `openrouter`. A user configures only some
of them. Does the picker list a provider the user cannot authenticate?

`SPEC-switch-the-provider-mid-session` did not settle this. Its `provider_names` "resolves no
credential", and it lists each built-in and each trusted entry. A reviewer then found the
result: every compiled built-in appears, so the picker fills with credential-error rows.

An implementer read the same spec and reported the gap. The spec must not be guessed at, so
this decision settles it.

## The decision

**rho lists a provider only when the configuration names a credential for it.**

The check reads the configuration. It does not resolve the credential value. So it never runs
a `!command` credential, it never reads a key file, and it makes no network call.

A provider with a named credential is listed. Its listing may still fail later, and
`D-a-provider-switch-that-cannot-build-is-refused` governs that case.

A provider with no named credential is absent from the picker. rho makes no call for it.

## The reason

The user named pi as the design they want. pi does this already, and the behaviour is
measured, not remembered. `docs/models.md` in the installed pi says it "treats models as
requiring auth before they appear in `/model`". It tells a user with a keyless local server to
keep a dummy key. See `.rho-work/prior-art-pi-provider-switch.md` section 5.

The alternative costs the user twice. It fills the picker with rows that cannot be chosen. It
also spends a credential read, and possibly a shell subprocess, on a provider the user never
configured.

This keeps `provider_names` cheap, which its own documentation promises. A configuration
lookup is a map read. A credential resolution is not.

## What this rules out

- A picker row for a provider the user never configured.
- A credential resolution for a provider the user is not using.
- A network call to a provider the user cannot authenticate.
- An error row as the normal way to report an unconfigured provider. An error row is for a
  provider that was configured and then failed.

## What this does not rule out

- A listed provider that fails at listing time. A key may be wrong, or a service may be down.
  That is a real error, and it draws one line for that provider.
- A future flag that lists every compiled provider, for a user who wants to see what exists.
  No such flag is built, and none is planned here.

## Test cases

- `a_provider_with_no_named_credential_is_not_listed` — the picker omits it, and no call is made.
- `a_provider_with_a_named_credential_is_listed` — presence in the configuration is enough.
- `provider_names_resolves_no_credential_value` — the check reads the configuration only.
