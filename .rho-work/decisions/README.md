# rho decisions

One decision, one file. A shared append-only file made two agents in two
worktrees collide on every entry, so each decision now owns its own file.

The reference form is `D-<slug>`. `docs/ids.md` maps every old numeric id.

| Decision | Date | Title |
| --- | --- | --- |
| [D-a-bad-middle-record-is-skipped-and-counted](20260822-093000-D-a-bad-middle-record-is-skipped-and-counted.md) | 20260822 | A bad record in the middle of a session file is skipped and counted |
| [D-a-bad-session-file-is-one-row](20260824-221849-D-a-bad-session-file-is-one-row.md) | 20260824 | One unreadable session file must not fail a list |
| [D-a-budget-is-measured-not-asserted](20260818-120304-D-a-budget-is-measured-not-asserted.md) | 20260818 | A cost budget is measured, or it is prose |
| [D-a-command-is-strict-and-an-event-is-loose](20260826-184416-D-a-command-is-strict-and-an-event-is-loose.md) | 20260826 | A command denies an unknown field, and an event ignores one |
| [D-a-command-line-is-capped](20260826-184415-D-a-command-line-is-capped.md) | 20260826 | One command line is capped, because stdin is the third unbounded reader |
| [D-a-dialog-answer-holds-exactly-one-value](20260826-184413-D-a-dialog-answer-holds-exactly-one-value.md) | 20260826 | A dialog answer holds exactly one value, and an untagged reader cannot |
| [D-a-dialog-timeout-cancels](20260826-184414-D-a-dialog-timeout-cancels.md) | 20260826 | A dialog timeout denies, and it never picks an option |
| [D-a-live-session-holds-a-lock](20260824-224512-D-a-live-session-holds-a-lock.md) | 20260824 | A live session holds an advisory lock, because fx is the only prior art that solved this |
| [D-a-merged-value-error-names-no-file](20260821-231500-D-a-merged-value-error-names-no-file.md) | 20260821 | A merged value error names the key and the value, and never a file |
| [D-a-panel-nobody-can-open](20260818-171500-D-a-panel-nobody-can-open.md) | 20260818 | A rendered panel with no key that opens it is not a feature |
| [D-a-project-key-cannot-escape-the-store](20260824-223011-D-a-project-key-cannot-escape-the-store.md) | 20260824 | A hostile repository must not choose where rho writes |
| [D-a-record-id-is-minted-against-the-set](20260824-221848-D-a-record-id-is-minted-against-the-set.md) | 20260824 | A record id is minted against the set, because a counter mints a duplicate |
| [D-a-rejected-definition-is-reported](20260826-184108-D-a-rejected-definition-is-reported.md) | 20260826 | A definition file that does not load says so |
| [D-a-role-column-is-not-a-stack](20260818-181500-D-a-role-column-is-not-a-stack.md) | 20260818 | A theme role resolves in one mode, and the columns never stack |
| [D-a-row-pattern-names-every-field](20260829-113201-D-a-row-pattern-names-every-field.md) | 20260829 | A renderer pattern over a row names every field, so the compiler catches the next one |
| [D-a-second-writer-can-delete-a-test-in-silence](20260822-131500-D-a-second-writer-can-delete-a-test-in-silence.md) | 20260822 | A second writer can delete a test in silence, so a commit's claims are checked |
| [D-a-session-file-is-private](20260824-223010-D-a-session-file-is-private.md) | 20260824 | The session store carries the transcript's mode bits |
| [D-a-session-id-sorts-by-time](20260824-221846-D-a-session-id-sorts-by-time.md) | 20260824 | A session id is a stamp and four hex characters, so a list sorts with no read |
| [D-a-session-title-costs-nothing](20260824-221851-D-a-session-title-costs-nothing.md) | 20260824 | A title comes from the first prompt or a name record, and never from a model |
| [D-a-stale-result-handle-expires-on-resume](20260824-221855-D-a-stale-result-handle-expires-on-resume.md) | 20260824 | A stale result handle expires on resume, and the model is told |
| [D-a-steer-is-never-rejected-for-being-early](20260826-184412-D-a-steer-is-never-rejected-for-being-early.md) | 20260826 | A steer is never rejected for being early |
| [D-a-steering-message-is-bounded-by-bytes](20260826-184207-D-a-steering-message-is-bounded-by-bytes.md) | 20260826 | The steering queue counts bytes, not only messages |
| [D-a-tool-keyword-stands-alone](20260820-115845-D-a-tool-keyword-stands-alone.md) | 20260820 | A tool keyword stands alone, and a mixed line keeps the names |
| [D-a-tool-list-accepts-a-yaml-sequence](20260826-184109-D-a-tool-list-accepts-a-yaml-sequence.md) | 20260826 | Both spellings of a tool list are right |
| [D-a-waiter-has-a-deadline](20260826-184208-D-a-waiter-has-a-deadline.md) | 20260826 | A queued child stops waiting, and the parent's turn returns |
| [D-acp-cancelled-spelling](20260817-171334-D-acp-cancelled-spelling.md) | 20260817 | The ACP cancelled stop reason needs an explicit serde rename |
| [D-acp-is-real-acp](20260817-164906-D-acp-is-real-acp.md) | 20260817 | The headless frontend is ACP, and it is the real ACP |
| [D-agentic-workflow-is-a-template](20260820-120345-D-agentic-workflow-is-a-template.md) | 20260820 | One agentic workflow template, filled per unit of work |
| [D-alternate-screen-after-all](20260819-093316-D-alternate-screen-after-all.md) | 20260819 | rho takes the whole screen, and it gives the transcript back with one key |
| [D-an-agent-symlink-cannot-smuggle-trust](20260826-201530-D-an-agent-symlink-cannot-smuggle-trust.md) | 20260826 | One loader, one classification |
| [D-an-error-on-the-event-stream-ends-the-run](20260826-193044-D-an-error-on-the-event-stream-ends-the-run.md) | 20260826 | An error on the event stream ends the run, because waiting is a hang |
| [D-append-only-jsonl](20260818-014343-D-append-only-jsonl.md) | 20260818 | The session file is append-only JSONL with a per-record parent pointer |
| [D-approval-default-ask](20260818-020639-D-approval-default-ask.md) | 20260818 | The approval default is Ask where answerable, read-only where not |
| [D-approval-option-not-enum](20260818-025953-D-approval-option-not-enum.md) | 20260818 | The resolved approval mode is an Option, so an unset value stays unset |
| [D-ask-policy-fails-closed](20260818-020639-D-ask-policy-fails-closed.md) | 20260818 | The Ask policy asks a frontend over a channel and fails closed |
| [D-bash-line-cap](20260817-190542-D-bash-line-cap.md) | 20260817 | The `bash` reader caps a single line |
| [D-bash-no-path-confinement](20260817-210307-D-bash-no-path-confinement.md) | 20260817 | `bash` path confinement stays out, and the reason is written down |
| [D-bash-os-sandbox](20260817-234608-D-bash-os-sandbox.md) | 20260817 | `bash` gets a real OS sandbox, and it fails closed |
| [D-bash-scrubs-credentials](20260817-201800-D-bash-scrubs-credentials.md) | 20260817 | `bash` scrubs credential variables, and the spec stops overclaiming |
| [D-benchmarks-owner](20260817-164906-D-benchmarks-owner.md) | 20260817 | `docs/benchmarks.md` is created by S11 and nothing may pre-empt it |
| [D-bounded-steering-queue](20260818-014343-D-bounded-steering-queue.md) | 20260818 | The steering queue is bounded, and a cancel keeps it |
| [D-budget-governor](20260817-235346-D-budget-governor.md) | 20260817 | A fleet-wide budget governor, and it is the safety valve for subagents |
| [D-cancel-keeps-the-session-open](20260818-014343-D-cancel-keeps-the-session-open.md) | 20260818 | Cancel reuses the existing CancelToken and keeps the session open |
| [D-cancel-wake-race](20260817-175024-D-cancel-wake-race.md) | 20260817 | Fix the `CancelToken::cancelled` wake race |
| [D-cap-a-large-tool-result](20260818-014343-D-cap-a-large-tool-result.md) | 20260818 | A large tool result is capped in the record, not stored verbatim |
| [D-chain-records-are-frozen](20260824-221847-D-chain-records-are-frozen.md) | 20260824 | A chain record set is frozen, a leaf record may be new, and an orphan is refused |
| [D-child-confined-by-composition](20260818-000223-D-child-confined-by-composition.md) | 20260818 | A child is confined by composition, not by comparison |
| [D-concise-mode-opt-in](20260818-090746-D-concise-mode-opt-in.md) | 20260818 | concise mode is opt-in and off by default |
| [D-config-fails-closed](20260818-014343-D-config-fails-closed.md) | 20260818 | Config fails closed on a malformed file, an unknown key, or an unreadable file |
| [D-confine-needs-a-traversal-test](20260818-115706-D-confine-needs-a-traversal-test.md) | 20260818 | A path boundary is tested with a traversal, not a sibling |
| [D-continue-is-scoped-to-the-project](20260824-221853-D-continue-is-scoped-to-the-project.md) | 20260824 | Continue takes the newest session for the project, across worktrees |
| [D-core-event-model-widens](20260817-170455-D-core-event-model-widens.md) | 20260817 | The architect may widen the core event model to fit ACP |
| [D-credential-command-allowlist](20260818-014343-D-credential-command-allowlist.md) | 20260818 | A credential command inherits an allowlist, stricter than the bash denylist |
| [D-duration-rounds-once](20260818-090744-D-duration-rounds-once.md) | 20260818 | the duration ladder rounds exactly once |
| [D-jcode-bash-lessons](20260817-231304-D-jcode-bash-lessons.md) | 20260817 | What reading jcode's `bash` tool changed |
| [D-jcode-edit-lessons](20260817-224427-D-jcode-edit-lessons.md) | 20260817 | What reading jcode's `edit` tool changed |
| [D-ledger-wins-the-band](20260818-215406-D-ledger-wins-the-band.md) | 20260818 | The band stays a fixed fourteen rows, and an approval never yields |
| [D-log-capture-proves-itself](20260818-060126-D-log-capture-proves-itself.md) | 20260818 | A log capture in a test must prove itself first |
| [D-mcp-does-not-classify-itself](20260817-215003-D-mcp-does-not-classify-itself.md) | 20260817 | An MCP server does not classify its own tools |
| [D-mcp-shared-by-default](20260817-215003-D-mcp-shared-by-default.md) | 20260817 | An MCP server is shared between sessions by default |
| [D-measured-codec-choice](20260818-014343-D-measured-codec-choice.md) | 20260818 | A JSONL codec choice is measured, and the fast codec stays optional |
| [D-measured-cost-and-cache](20260817-234608-D-measured-cost-and-cache.md) | 20260817 | Report the cache hit rate and the real cost, because a competitor only claims them |
| [D-no-caller-writes-a-session-file](20260821-234500-D-no-caller-writes-a-session-file.md) | 20260821 | No production caller writes a session file, and the persisted format has no reader |
| [D-no-cross-session-cache](20260817-235346-D-no-cross-session-cache.md) | 20260817 | No cross-session shared file cache, because rho has no tenancy model |
| [D-no-event-carries-the-success-key](20260826-190512-D-no-event-carries-the-success-key.md) | 20260826 | No event carries the success key, because a client routes on it |
| [D-no-four-argument-session-new](20260817-180735-D-no-four-argument-session-new.md) | 20260817 | Remove the four-argument `Session::new` |
| [D-no-git-writes-by-a-subagent](20260817-184709-D-no-git-writes-by-a-subagent.md) | 20260817 | A subagent must never run a git command that changes the working tree |
| [D-no-list-cache-until-a-budget-fails](20260824-221850-D-no-list-cache-until-a-budget-fails.md) | 20260824 | No list cache ships until a measured budget fails |
| [D-no-remembered-execute-allow](20260818-020639-D-no-remembered-execute-allow.md) | 20260818 | A remembered allow never covers a tool that runs a program |
| [D-one-redaction-home](20260817-222057-D-one-redaction-home.md) | 20260817 | Redaction has one home, because the copies had already drifted |
| [D-one-timestamp-format](20260818-060126-D-one-timestamp-format.md) | 20260818 | One session file holds one timestamp format, and it is epoch milliseconds |
| [D-own-session-format](20260817-164906-D-own-session-format.md) | 20260817 | Session file format is rho's own, not pi's |
| [D-plugin-does-not-classify-itself](20260817-190542-D-plugin-does-not-classify-itself.md) | 20260817 | A plugin does not classify itself |
| [D-plugin-trust-policy](20260817-210307-D-plugin-trust-policy.md) | 20260817 | The plugin host states a trust policy, and refuses a plugin in the session root |
| [D-progress-follows-the-state-and-never-moves-it](20260829-113200-D-progress-follows-the-state-and-never-moves-it.md) | 20260829 | The task row draws its progress last, and a value that changes never moves one that does not |
| [D-project-skill-needs-trust](20260817-215003-D-project-skill-needs-trust.md) | 20260817 | A project skill is not loaded until the project is trusted |
| [D-provider-contract-crate](20260817-175201-D-provider-contract-crate.md) | 20260817 | The provider contract suite is a real crate, not a private test file |
| [D-provider-extension-verified-outside](20260817-200505-D-provider-extension-verified-outside.md) | 20260817 | The provider extension point is verified from outside the workspace |
| [D-pty-teardown-closes-before-it-waits](20260818-155000-D-pty-teardown-closes-before-it-waits.md) | 20260818 | A pty harness closes the master, then kills, then waits with a deadline |
| [D-read-only-maps-onto-approval](20260820-214512-D-read-only-maps-onto-approval.md) | 20260820 | `--read-only` writes the approval key, and only when true |
| [D-reader-line-cap](20260818-020639-D-reader-line-cap.md) | 20260818 | The session reader caps one line |
| [D-reasoning-replay-is-opaque-provider-state](20260821-220620-D-reasoning-replay-is-opaque-provider-state.md) | 20260821 | A provider replays its own reasoning through one opaque, owner-tagged state value |
| [D-recorder-consumes-events](20260818-014343-D-recorder-consumes-events.md) | 20260818 | The session log is an event-stream consumer, not a field in Session |
| [D-recording-is-on-by-default](20260824-221852-D-recording-is-on-by-default.md) | 20260824 | Recording is on by default, because an unwritten session cannot be resumed |
| [D-redact-json-secrets](20260818-020639-D-redact-json-secrets.md) | 20260818 | Session redaction uses one new function in rho-redact |
| [D-redact-tool-arguments](20260818-014343-D-redact-tool-arguments.md) | 20260818 | A tool argument is redacted on the way into the file |
| [D-reopen-stated-on-disk](20260818-060126-D-reopen-stated-on-disk.md) | 20260818 | A reopen is stated on disk, and every io error names its path |
| [D-replay-only-the-current-loop](20260822-093100-D-replay-only-the-current-loop.md) | 20260822 | Only the current tool loop replays its reasoning |
| [D-resume-is-a-flag-on-run](20260824-221854-D-resume-is-a-flag-on-run.md) | 20260824 | One session flag on run, two spellings, and an optional value |
| [D-resume-never-widens](20260818-020639-D-resume-never-widens.md) | 20260818 | A resume must not widen a permission |
| [D-retention-is-not-in-the-wiring-lane](20260824-221856-D-retention-is-not-in-the-wiring-lane.md) | 20260824 | Retention waits, because a config key that nothing reads is dead surface |
| [D-retry-numbers-configurable](20260818-001918-D-retry-numbers-configurable.md) | 20260818 | Retry is configurable in its numbers and fixed in its rules |
| [D-rho-jsonl-asks-a-factory-for-a-session](20260826-184606-D-rho-jsonl-asks-a-factory-for-a-session.md) | 20260826 | The JSONL frontend asks a factory for a session, and links no provider |
| [D-sandbox-is-correctness](20260817-235346-D-sandbox-is-correctness.md) | 20260817 | The bash sandbox is a correctness win, not a structural lead |
| [D-secret-in-core](20260817-181635-D-secret-in-core.md) | 20260817 | `Secret` and `RetryPolicy` belong in `rho-core` |
| [D-serde-json-default-codec](20260818-014343-D-serde-json-default-codec.md) | 20260818 | serde_json is the default codec, and sonic-rs is an off-by-default feature |
| [D-session-config](20260817-175834-D-session-config.md) | 20260817 | `Session` needs a `SessionConfig` |
| [D-session-context-accessor](20260817-175024-D-session-context-accessor.md) | 20260817 | `Session` gets a read-only context accessor |
| [D-session-store-layout](20260824-221845-D-session-store-layout.md) | 20260824 | A session lives under a project key that every worktree shares |
| [D-settled-is-the-only-end-signal](20260826-184410-D-settled-is-the-only-end-signal.md) | 20260826 | Settled is the only end signal, so there is no RunEnd |
| [D-seven-column-duration-slot](20260818-090745-D-seven-column-duration-slot.md) | 20260818 | every duration sits in a seven-column slot |
| [D-shared-working-tree](20260817-224703-D-shared-working-tree.md) | 20260817 | Another agent shares this working tree, so read the diff before you commit |
| [D-skill-allowed-tools-ignored](20260817-215003-D-skill-allowed-tools-ignored.md) | 20260817 | `allowed-tools` in a skill is parsed, warned about, and ignored |
| [D-slug-ids](20260818-041358-D-slug-ids.md) | 20260818 | An artifact is named by a timestamp and a slug, never by a counter |
| [D-sprint-one-hook-interfaces](20260817-170455-D-sprint-one-hook-interfaces.md) | 20260817 | The Hook trait and the approval gate are sprint-1 interfaces |
| [D-staged-server-serves-every-connection](20260818-043346-D-staged-server-serves-every-connection.md) | 20260818 | A test double must answer every connection |
| [D-the-frontend-settles-every-prompt](20260826-184411-D-the-frontend-settles-every-prompt.md) | 20260826 | The frontend settles every accepted prompt, because rho-core does not |
| [D-the-merge-cannot-name-a-values-source](20260820-214513-D-the-merge-cannot-name-a-values-source.md) | 20260820 | The merge loses which layer held a bad value |
| [D-the-record-id-is-visible-on-the-command-line](20260824-224710-D-the-record-id-is-visible-on-the-command-line.md) | 20260824 | A fork needs a record id that a user can actually see |
| [D-the-wheel-needs-capture](20260819-120429-D-the-wheel-needs-capture.md) | 20260819 | rho captures the mouse, because the wheel is the only way to scroll |
| [D-the-wire-reuses-the-core-stop-reason](20260826-184409-D-the-wire-reuses-the-core-stop-reason.md) | 20260826 | The wire reuses the core stop reason, and keeps no copy |
| [D-three-reasoning-costs-stay-open](20260822-101500-D-three-reasoning-costs-stay-open.md) | 20260822 | Three reasoning costs stay open, and each one names its trigger |
| [D-three-tiers](20260817-232444-D-three-tiers.md) | 20260817 | Three tiers, named for what each contributes |
| [D-todo-in-a-green-stage](20260817-175834-D-todo-in-a-green-stage.md) | 20260817 | Three `todo!()` bodies survived stage S4, and one is a security boundary |
| [D-truncated-tail-warns](20260818-014343-D-truncated-tail-warns.md) | 20260818 | Resume drops a truncated last line and warns, and keeps every whole record |
| [D-tui-plugin-is-a-tier-2-trait](20260818-055346-D-tui-plugin-is-a-tier-2-trait.md) | 20260818 | The terminal view surface is a Tier-2 in-tree trait |
| [D-tui-plugin-render-budget](20260818-055346-D-tui-plugin-render-budget.md) | 20260818 | A terminal view render has an 8 ms budget and a worker thread |
| [D-tui-plugin-trust-default-no-transcript](20260818-055346-D-tui-plugin-trust-default-no-transcript.md) | 20260818 | A terminal view reads nothing until the user opts in |
| [D-two-variants-cannot-share-a-serde-tag](20260821-221500-D-two-variants-cannot-share-a-serde-tag.md) | 20260821 | Two enum variants cannot share one serde tag, and the reader says nothing |
| [D-two-weak-tests](20260818-003819-D-two-weak-tests.md) | 20260818 | Two weak tests found by breaking the code, and one design consequence recorded |
| [D-unmappable-pi-record-drops](20260818-030907-D-unmappable-pi-record-drops.md) | 20260818 | An unmappable pi record drops with a count, and the import finishes |
| [D-website-direction](20260817-170455-D-website-direction.md) | 20260817 | Website direction is mockup C, with A's command-prompt labels |
| [D-write-failure-degrades](20260818-014343-D-write-failure-degrades.md) | 20260818 | A write failure degrades a session to ephemeral, and never ends the run |
| [D-writer-holds-one-sink](20260818-060126-D-writer-holds-one-sink.md) | 20260818 | The writer holds one sink, and a test injects a failing sink |
