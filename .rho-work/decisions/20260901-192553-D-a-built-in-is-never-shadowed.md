# D-a-built-in-is-never-shadowed — refuse a collision, not an override

**Question:** a config entry names `id = "bedrock"`. Does it override the built-in, error out, or shadow only after the built-ins are checked?

**Decision: a collision is refused at load time.** A `[[providers]]` entry with the same id as a built-in returns `ConfigError::BuiltinCollision`. The built-ins are never shadowed.

## The reason

A built-in provider is safe, reviewed code. A named entry carries a base URL the user set. An override redirects the credential.

**Fail-safe means refuse the redirect.** A silent override sends the Bedrock credential to a user-set endpoint. That reads as a mistake or an attack. An error names the collision. It tells the user to rename the entry.

## What this rules out

**A named entry overriding a built-in.** The credential would travel somewhere the built-in name does not promise.

**A search-order shadow, built-ins first.** A config entry named `bedrock` would load and do nothing, which teaches the user nothing.

**A search-order shadow, named entries first.** A user could override a built-in by accident. The credential would travel without a warning.

## What this allows

A user who wants the name `bedrock` for a custom entry may not have it. They rename the entry to `my-bedrock` or `bedrock-local`. The collision error tells them why.
