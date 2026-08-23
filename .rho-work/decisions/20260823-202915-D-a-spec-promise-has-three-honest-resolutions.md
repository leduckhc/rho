# D-a-spec-promise-has-three-honest-resolutions — how to clear a check-spec-tests violation

**Question:** `bench/check-spec-tests.py` says a delivered spec names a test that no code
defines. The guard offers three exits: write the test, correct the name, or mark the line
planned. Which one applies, and who decides?

**Decision:** the reason for the gap decides, and the three exits are not interchangeable.
Read the gap first, then pick.

- **The promise is real and nothing covers it. Write the test.**
  `every_scalar_key_merges_and_reaches_the_config` was this case. `ConfigLayer::merge`
  assigned fifteen fields by hand, and every other test read one key, so a forgotten line
  dropped a value in silence. The spec had named the guard and nobody wrote it.

- **A test covers the assertion under another name. Correct the spec.**
  `an_absent_credential_is_an_error_not_an_empty_key` was this case.
  `a_missing_env_credential_is_an_error` already proves it. The spec follows the code here,
  because the code is right and only the name drifted.

- **The rule the test would prove is unbuilt. Mark the line planned.**
  Rules 4 and 7 of `SPEC-config-call-site` are the stderr announcements, and the spec's own
  status says they stay unbuilt while question U4 is open. Three lines named their tests.
  A test cannot exist for behaviour that does not, so `planned` is the honest state.

**A retired name is a fourth case, and it is not a promise.** The spec said one test
"replaces `the_flag_wins_over_the_env_var`". The dead name was backticked, so the guard read
it as a promise. Prose about a retired test must not backtick the retired name. Name the old
shape in words instead.

**The marker goes on the line that holds the name.** The guard exempts a line that contains
the word `planned`, and it records the line where the name appears. A continuation line does
not count. Three violations survived a first fix for exactly that reason.

**Rules out:** marking a line planned to clear a violation when the behaviour ships. That
turns the guard off for the one case it exists to catch. Deleting a spec line to make the
gate pass, which loses the promise instead of keeping it. Writing a placeholder test with the
promised name and a weak body, which is worse than the violation, because the gate then reads
as green while nothing is proved.
