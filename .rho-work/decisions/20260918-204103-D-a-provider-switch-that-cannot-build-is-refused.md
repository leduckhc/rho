# D-a-provider-switch-that-cannot-build-is-refused

Date: 20260918-204103

## The question

A user picks a foreign model. The switch builds a new provider. The build can fail: a
credential is missing, a protocol is unknown, an untrusted project file named the
credential, or a provider refuses its base url. What does the session do when the switch
fails? And what stops a switch from sending a credential to a host the user did not
authorise?

## The decision

A switch that cannot build a provider is refused. The session keeps its current provider,
its current model, and its current effort. Nothing changes.

`Session::apply_selection` builds the new provider first, before it commits any state. On a
build failure it returns `Err(ProviderBuildError)` and mutates nothing: not the running
unit, and not the session file. On success it commits under one lock: it swaps the
provider, its name, the model, and the effort together, then records one `ModelChange` with
the new provider name.

So the order is build, then commit. A half-applied switch cannot happen, because the only
write follows a successful build, and the commit is one atomic lock hold.

### Who reports it

The core refuses. The frontend reports. `rho-tui` calls `apply_selection`, and on `Err` it
pushes one error row into the picker or the transcript, and it does not mirror the pick
into the state. So the header still names the running provider, which is the true one. The
error line is `ProviderBuildError`'s message, which holds no secret.

### The trust gate holds by reuse

The factory build is `build_provider`, the same function the session used at startup. So
every credential and trust rule holds identically for a switch:

- A missing credential names the variable to set, and refuses the build. See
  `D-a-provider-names-its-own-credential`.
- An untrusted project credential is refused, and the message names `--trust-project`. See
  `D-an-untrusted-clone-supplies-no-credential`.
- An unknown protocol and a base-url conflict each refuse with their own message.

A switch cannot reach a credential a startup could not reach, because it runs the same
gate. So the switch adds no new credential surface.

### The base-url redirect is gated and announced

A base url can redirect a credential. `build_provider` applies `config.base_url` to the
OpenAI-compatible providers, so a switch to such a provider inherits that base url and
sends the key there. See `D-a-provider-base-url-is-a-config-key`.

Two rules already block the escalation, and a switch inherits both. First, `rho-config`
drops an untrusted project `base-url`, so a clone cannot set one. Second, a provider that
names its endpoint its own way refuses a base url with `IncompatibleBaseUrl`. So a switch
cannot send a key to a host an untrusted file chose.

A switch also announces a redirect the way startup announces it. When a base url is set,
`rho` shows where the key goes. A live switch shows the same notice, so the user always
sees the key's destination. A switch never sends a credential to an un-announced host.

### The fail-closed rule

There is no fallback provider and no default on failure. A session with no injected factory
refuses every switch, rather than silently building a default. An unbuildable provider name
is a hard error, not an `Unknown` that grants a default capability. A fail-open default is
this project's most-repeated defect, so the switch path has none.

## What this rules out

- **A half-applied switch.** The build precedes the atomic commit, so a failure leaves no
  partial state.
- **A silent fallback to a default provider.** A failure is reported, never hidden.
- **A drift between the header and the wire.** On failure the state is not mirrored, so the
  header keeps naming the running provider.
- **A new credential surface.** The switch reuses `build_provider`, so it runs the same
  gate as startup.
- **A credential sent to an un-announced host.** A switch inherits the base-url gate and
  the redirect notice.
- **A secret in the error.** `ProviderBuildError` holds one line from a `Display` that never
  carries a credential value.

## Why

The session must survive a bad switch. The safe outcome is the running provider, untouched.
Build first and commit second makes that outcome automatic, and reusing `build_provider`
keeps one credential gate and one base-url gate for both startup and a live switch.
