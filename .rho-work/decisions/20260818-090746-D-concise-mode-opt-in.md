# D-concise-mode-opt-in — concise mode is opt-in and off by default

**Question.** Does concise mode collapse a tool row by default, or only when the user asks?

**Decision.** Concise mode is opt-in. The default is off. The setting is `tui.concise`. A
failed tool row still expands itself, whatever the setting, because its output is the point.

**Reason.** A new user must see the tool output to trust the harness. A hidden body on the
first run reads as a missing feature, not a clean one. So the body shows by default, and
the user turns concise mode on once they want the density.

**It rules out.** It rules out a collapsed-by-default tool row. It rules out a mode a user
must find before they see any output. It rules out hiding a failed row's output.
