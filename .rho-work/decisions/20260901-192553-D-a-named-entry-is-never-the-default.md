# D-a-named-entry-is-never-the-default — keep the build default

**Question:** a config file defines one `[[providers]]` entry. The user sets no `--provider` flag. Does the named entry become the default?

**Decision: no. The default is the build default.** With no flag and no `RHO_PROVIDER` variable, rho uses the build default. That is `openrouter` when that feature is compiled. A named entry never becomes the default.

## The reason

A default is a convenience, not a policy. A built-in default is shipped code the user did not choose. That is already a tradeoff.

**An entry in a config file is even less of a choice.** A project file arrives with a clone. An entry there could redirect the credential with no flag. A global-file entry is the user's own, but it is still a file edit the user may not remember.

The CLI already has one default: the build default. A second source of default makes the result depend on which file exists. That confuses "I set nothing" with "I set something once and forgot".

## What this rules out

**Using a named entry as the default when exactly one exists.** That makes the default depend on the file content. The file is silent and far from the prompt.

**Preferring a named entry over the build default.** That redirects the credential with no flag in a cloned project.

## What this allows

A named entry is always explicit. The user says `--provider xdent-claude` or sets `RHO_PROVIDER=xdent-claude`. A config entry buys a short name, not a default.
