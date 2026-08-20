# D-project-skill-needs-trust — A project skill is not loaded until the project is trusted


A skill is instructions the model will follow, and it may carry scripts the model will
run. So a skill inside the repository under edit is a prompt injection with a filename.

This is the same threat as decision D-plugin-trust-policy, where the plugin host refuses a plugin under
the session root. A skill is worse in one way: nobody reads a Markdown file as code.

**Decision.** A project skill is withheld until the user trusts that session root. The
default is untrusted, and nothing infers trust. A withheld skill is still **listed**,
because a user who cannot see a skill cannot decide about it.

`--read-only` does not grant skill trust. The two are independent. A read-only session
still follows instructions, and instructions can exfiltrate through a read.
