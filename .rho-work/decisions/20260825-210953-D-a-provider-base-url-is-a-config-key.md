# D-a-provider-base-url-is-a-config-key — one key, and every OpenAI-compatible host

**Question:** `OpenRouterConfig` carries a settable `base_url`, and nothing in the command
line or the config reaches it. Ollama, vLLM, LiteLLM, and LM Studio all speak the same wire
format. How should a user point rho at one?

**Decision:** a config key, `base-url`, with `RHO_BASE_URL` beside it. The OpenRouter provider
already is the OpenAI-compatible client, so no new crate and no new provider name.

This is the cheapest capability on rho's whole backlog. One key turns a three-provider harness
into one that talks to any local model host.

## The credential is the hard part

A base url change redirects **your key**. Point `base-url` at a host you do not control, and
rho sends `OPENROUTER_API_KEY` to it. That is credential exfiltration through a config file,
and a project-level config file is attacker-controlled input in a cloned repository.

Three rules, and each fails closed.

- **A project config file may not set `base-url` unless the project is trusted.** It joins
  `skill-paths`, `mcp-config`, and a `!command` credential in the untrusted-drop list. A
  cloned repository cannot redirect your key by itself.
- **A base url must be `https`, or it must be a loopback host.** `http://localhost:11434` is
  how Ollama works, and that traffic never leaves the machine. Plain `http` to any other host
  would put the key on the wire in clear text, so rho refuses it and names the reason.
- **rho says where the key is going.** A startup notice names the host whenever `base-url` is
  set. A silent redirect is the defect; a loud one is a feature.

## Rules out

**A new provider name per host.** `--provider ollama` would need a crate, a credential
variable, and a default model for every host anyone runs. The wire format is the same, so the
provider is the same.

**Reusing `--provider` as a url.** It would make one flag mean two things, and a typo would
read as a host rather than an unknown provider.

**Silently trusting a project file.** See the exfiltration path above.

**A per-host key map.** `[credentials]` already reaches nothing, so building on it would put a
feature on a surface that does not work. When credentials reach providers, a host-keyed entry
becomes the natural next step, and this decision does not block it.

## Rules that hold

- With no `base-url`, nothing changes. The default stays the OpenRouter endpoint.
- The key stays `OPENROUTER_API_KEY`, because the provider is unchanged. The notice says so.
- A model id is passed through, as it is today. A local host names its own models.
