# The id registry

rho names an artifact by a timestamp and a slug. It does not number artifacts, because a
number needs a counter, and a counter clashes whenever two worktrees allocate the next
value at the same time. This project hit that clash on every parallel stage.

This page is permanent. Fifty commit messages, and every earlier report, name the old
numeric ids. So the mapping must stay readable for as long as the history matters.

**The scheme.**

| Kind | File | Reference |
| --- | --- | --- |
| Spec | `docs/specs/<yyyymmdd-hhmmss>-SPEC-<slug>.md` | `SPEC-<slug>` |
| ADR | `docs/adr/<yyyymmdd-hhmmss>-ADR-<slug>.md` | `ADR-<slug>` |
| Decision | `.rho-work/decisions/<yyyymmdd-hhmmss>-D-<slug>.md` | `D-<slug>` |
| Feature | one row in `docs/features.md` | `F-<slug>` |

The timestamp is the moment the artifact first entered git. It makes a file name unique
and it sorts by age. The slug is the reference, because a reader should not need a lookup.

`bench/check-ids.py` enforces every rule here. It fails on a numeric id, on a reference
that resolves to nothing, and on a duplicate slug.

**To add an artifact.** Take the current UTC time with `date -u +%Y%m%d-%H%M%S`, pick a
slug that says what the artifact decides, and write the file. Ask nobody for a number.


## Specs

| Old | Reference | File |
| --- | --- | --- |
| `SPEC-01` | `SPEC-core-runtime` | `docs/specs/20260817-164906-SPEC-core-runtime.md` |
| `SPEC-02` | `SPEC-provider-interface` | `docs/specs/20260817-164906-SPEC-provider-interface.md` |
| `SPEC-03` | `SPEC-tool-interface` | `docs/specs/20260817-164906-SPEC-tool-interface.md` |
| `SPEC-04` | `SPEC-hooks-and-plugins` | `docs/specs/20260817-170455-SPEC-hooks-and-plugins.md` |
| `SPEC-05` | `SPEC-tui` | `docs/specs/20260817-170455-SPEC-tui.md` |
| `SPEC-06` | `SPEC-acp` | `docs/specs/20260817-170455-SPEC-acp.md` |
| `SPEC-07` | `SPEC-background-tasks` | `docs/specs/20260817-210307-SPEC-background-tasks.md` |
| `SPEC-08` | `SPEC-skills` | `docs/specs/20260817-215003-SPEC-skills.md` |
| `SPEC-09` | `SPEC-mcp` | `docs/specs/20260817-215003-SPEC-mcp.md` |
| `SPEC-10` | `SPEC-bash-sandbox` | `docs/specs/20260817-234951-SPEC-bash-sandbox.md` |
| `SPEC-11` | `SPEC-subagents` | `docs/specs/20260818-000223-SPEC-subagents.md` |
| `SPEC-12` | `SPEC-budget-governor` | `docs/specs/20260817-235346-SPEC-budget-governor.md` |
| `SPEC-13` | `SPEC-config` | `docs/specs/20260818-014343-SPEC-config.md` |
| `SPEC-14` | `SPEC-sessions` | `docs/specs/20260818-014343-SPEC-sessions.md` |
| `SPEC-15` | `SPEC-steering` | `docs/specs/20260818-014343-SPEC-steering.md` |
| `SPEC-16` | `SPEC-approval` | `docs/specs/20260818-020639-SPEC-approval.md` |

## ADRs

| Old | Reference | File |
| --- | --- | --- |
| `ADR-001` | `ADR-plugin-mechanism` | `docs/adr/20260817-170455-ADR-plugin-mechanism.md` |
| `ADR-002` | `ADR-footprint` | `docs/adr/20260817-170455-ADR-footprint.md` |
| `ADR-003` | `ADR-event-model` | `docs/adr/20260817-170455-ADR-event-model.md` |
| `ADR-004` | `ADR-session-format` | `docs/adr/20260818-014343-ADR-session-format.md` |
| `ADR-005` | `ADR-jsonl-codec` | `docs/adr/20260818-014343-ADR-jsonl-codec.md` |

## Decisions

| Old | Reference | File |
| --- | --- | --- |
| `D-001` | `D-own-session-format` | `.rho-work/decisions/20260817-164906-D-own-session-format.md` |
| `D-002` | `D-acp-is-real-acp` | `.rho-work/decisions/20260817-164906-D-acp-is-real-acp.md` |
| `D-003` | `D-benchmarks-owner` | `.rho-work/decisions/20260817-164906-D-benchmarks-owner.md` |
| `D-004` | `D-website-direction` | `.rho-work/decisions/20260817-170455-D-website-direction.md` |
| `D-005` | `D-sprint-one-hook-interfaces` | `.rho-work/decisions/20260817-170455-D-sprint-one-hook-interfaces.md` |
| `D-006` | `D-core-event-model-widens` | `.rho-work/decisions/20260817-170455-D-core-event-model-widens.md` |
| `D-007` | `D-acp-cancelled-spelling` | `.rho-work/decisions/20260817-171334-D-acp-cancelled-spelling.md` |
| `D-008` | `D-session-context-accessor` | `.rho-work/decisions/20260817-175024-D-session-context-accessor.md` |
| `D-009` | `D-cancel-wake-race` | `.rho-work/decisions/20260817-175024-D-cancel-wake-race.md` |
| `D-010` | `D-provider-contract-crate` | `.rho-work/decisions/20260817-175201-D-provider-contract-crate.md` |
| `D-011` | `D-session-config` | `.rho-work/decisions/20260817-175834-D-session-config.md` |
| `D-012` | `D-todo-in-a-green-stage` | `.rho-work/decisions/20260817-175834-D-todo-in-a-green-stage.md` |
| `D-013` | `D-no-four-argument-session-new` | `.rho-work/decisions/20260817-180735-D-no-four-argument-session-new.md` |
| `D-014` | `D-secret-in-core` | `.rho-work/decisions/20260817-181635-D-secret-in-core.md` |
| `D-015` | `D-no-git-writes-by-a-subagent` | `.rho-work/decisions/20260817-184709-D-no-git-writes-by-a-subagent.md` |
| `D-016` | `D-bash-line-cap` | `.rho-work/decisions/20260817-190542-D-bash-line-cap.md` |
| `D-017` | `D-plugin-does-not-classify-itself` | `.rho-work/decisions/20260817-190542-D-plugin-does-not-classify-itself.md` |
| `D-018` | `D-provider-extension-verified-outside` | `.rho-work/decisions/20260817-200505-D-provider-extension-verified-outside.md` |
| `D-019` | `D-bash-scrubs-credentials` | `.rho-work/decisions/20260817-201800-D-bash-scrubs-credentials.md` |
| `D-020` | `D-plugin-trust-policy` | `.rho-work/decisions/20260817-210307-D-plugin-trust-policy.md` |
| `D-021` | `D-bash-no-path-confinement` | `.rho-work/decisions/20260817-210307-D-bash-no-path-confinement.md` |
| `D-022` | `D-project-skill-needs-trust` | `.rho-work/decisions/20260817-215003-D-project-skill-needs-trust.md` |
| `D-023` | `D-skill-allowed-tools-ignored` | `.rho-work/decisions/20260817-215003-D-skill-allowed-tools-ignored.md` |
| `D-024` | `D-mcp-does-not-classify-itself` | `.rho-work/decisions/20260817-215003-D-mcp-does-not-classify-itself.md` |
| `D-025` | `D-mcp-shared-by-default` | `.rho-work/decisions/20260817-215003-D-mcp-shared-by-default.md` |
| `D-026` | `D-one-redaction-home` | `.rho-work/decisions/20260817-222057-D-one-redaction-home.md` |
| `D-027` | `D-jcode-edit-lessons` | `.rho-work/decisions/20260817-224427-D-jcode-edit-lessons.md` |
| `D-028` | `D-shared-working-tree` | `.rho-work/decisions/20260817-224703-D-shared-working-tree.md` |
| `D-029` | `D-jcode-bash-lessons` | `.rho-work/decisions/20260817-231304-D-jcode-bash-lessons.md` |
| `D-030` | `D-three-tiers` | `.rho-work/decisions/20260817-232444-D-three-tiers.md` |
| `D-031` | `D-bash-os-sandbox` | `.rho-work/decisions/20260817-234608-D-bash-os-sandbox.md` |
| `D-032` | `D-measured-cost-and-cache` | `.rho-work/decisions/20260817-234608-D-measured-cost-and-cache.md` |
| `D-033` | `D-budget-governor` | `.rho-work/decisions/20260817-235346-D-budget-governor.md` |
| `D-034` | `D-no-cross-session-cache` | `.rho-work/decisions/20260817-235346-D-no-cross-session-cache.md` |
| `D-035` | `D-sandbox-is-correctness` | `.rho-work/decisions/20260817-235346-D-sandbox-is-correctness.md` |
| `D-036` | `D-child-confined-by-composition` | `.rho-work/decisions/20260818-000223-D-child-confined-by-composition.md` |
| `D-037` | `D-retry-numbers-configurable` | `.rho-work/decisions/20260818-001918-D-retry-numbers-configurable.md` |
| `D-038` | `D-two-weak-tests` | `.rho-work/decisions/20260818-003819-D-two-weak-tests.md` |
| `D-039` | `D-append-only-jsonl` | `.rho-work/decisions/20260818-014343-D-append-only-jsonl.md` |
| `D-040` | `D-truncated-tail-warns` | `.rho-work/decisions/20260818-014343-D-truncated-tail-warns.md` |
| `D-041` | `D-write-failure-degrades` | `.rho-work/decisions/20260818-014343-D-write-failure-degrades.md` |
| `D-042` | `D-recorder-consumes-events` | `.rho-work/decisions/20260818-014343-D-recorder-consumes-events.md` |
| `D-043` | `D-redact-tool-arguments` | `.rho-work/decisions/20260818-014343-D-redact-tool-arguments.md` |
| `D-044` | `D-cap-a-large-tool-result` | `.rho-work/decisions/20260818-014343-D-cap-a-large-tool-result.md` |
| `D-045` | `D-serde-json-default-codec` | `.rho-work/decisions/20260818-014343-D-serde-json-default-codec.md` |
| `D-046` | `D-credential-command-allowlist` | `.rho-work/decisions/20260818-014343-D-credential-command-allowlist.md` |
| `D-047` | `D-config-fails-closed` | `.rho-work/decisions/20260818-014343-D-config-fails-closed.md` |
| `D-048` | `D-bounded-steering-queue` | `.rho-work/decisions/20260818-014343-D-bounded-steering-queue.md` |
| `D-049` | `D-cancel-keeps-the-session-open` | `.rho-work/decisions/20260818-014343-D-cancel-keeps-the-session-open.md` |
| `D-050` | `D-measured-codec-choice` | `.rho-work/decisions/20260818-014343-D-measured-codec-choice.md` |
| `D-051` | `D-approval-default-ask` | `.rho-work/decisions/20260818-020639-D-approval-default-ask.md` |
| `D-052` | `D-ask-policy-fails-closed` | `.rho-work/decisions/20260818-020639-D-ask-policy-fails-closed.md` |
| `D-053` | `D-resume-never-widens` | `.rho-work/decisions/20260818-020639-D-resume-never-widens.md` |
| `D-054` | `D-redact-json-secrets` | `.rho-work/decisions/20260818-020639-D-redact-json-secrets.md` |
| `D-055` | `D-reader-line-cap` | `.rho-work/decisions/20260818-020639-D-reader-line-cap.md` |
| `D-056` | `D-no-remembered-execute-allow` | `.rho-work/decisions/20260818-020639-D-no-remembered-execute-allow.md` |
| `D-057` | `D-approval-option-not-enum` | `.rho-work/decisions/20260818-025953-D-approval-option-not-enum.md` |
| `D-058` | `D-unmappable-pi-record-drops` | `.rho-work/decisions/20260818-030907-D-unmappable-pi-record-drops.md` |
| `D-059` | `D-writer-holds-one-sink` | `.rho-work/decisions/20260818-060126-D-writer-holds-one-sink.md` |
| `D-060` | `D-one-timestamp-format` | `.rho-work/decisions/20260818-060126-D-one-timestamp-format.md` |
| `D-061` | `D-reopen-stated-on-disk` | `.rho-work/decisions/20260818-060126-D-reopen-stated-on-disk.md` |
| `D-062` | `D-log-capture-proves-itself` | `.rho-work/decisions/20260818-060126-D-log-capture-proves-itself.md` |

## Features

| Old | Reference |
| --- | --- |
| `F-01` | `F-agent-loop` |
| `F-02` | `F-event-stream` |
| `F-03` | `F-turn-model` |
| `F-04` | `F-cancellation` |
| `F-05` | `F-auto-retry` |
| `F-06` | `F-auto-continue` |
| `F-07` | `F-background-tasks` |
| `F-08` | `F-message-queue` |
| `F-09` | `F-message-steering` |
| `F-10` | `F-provider-trait` |
| `F-100` | `F-structured-tracing` |
| `F-101` | `F-token-and-cost-accounting` |
| `F-102` | `F-session-statistics` |
| `F-103` | `F-no-secrets-in-logs` |
| `F-11` | `F-openrouter-provider` |
| `F-110` | `F-low-resident-memory-per-session` |
| `F-111` | `F-fast-cold-start` |
| `F-112` | `F-release-build-size` |
| `F-113` | `F-separate-feature-flags-for-providers` |
| `F-12` | `F-aws-bedrock-provider` |
| `F-120` | `F-single-binary` |
| `F-121` | `F-library-first-api` |
| `F-122` | `F-website` |
| `F-123` | `F-ci-pipeline` |
| `F-13` | `F-azure-openai-provider` |
| `F-130` | `F-approval-ui-extensions` |
| `F-14` | `F-model-registry` |
| `F-140` | `F-scoped-command-guardrails` |
| `F-141` | `F-default-destructive-shape-deny-list` |
| `F-142` | `F-narrower-write-confinement` |
| `F-143` | `F-budget-caps` |
| `F-144` | `F-web-fetch` |
| `F-145` | `F-web-search` |
| `F-146` | `F-browser-control` |
| `F-147` | `F-external-content-marking` |
| `F-148` | `F-task-list` |
| `F-149` | `F-task-confidence-scoring` |
| `F-15` | `F-custom-provider-extension` |
| `F-150` | `F-auto-continue-on-unfinished-work` |
| `F-151` | `F-durable-memory` |
| `F-152` | `F-language-server-tools` |
| `F-153` | `F-cost-meter` |
| `F-160` | `F-terminate-on-block` |
| `F-161` | `F-lifecycle-hook-points` |
| `F-162` | `F-model-request-and-response-hooks` |
| `F-163` | `F-slash-commands` |
| `F-170` | `F-subagent-spawning` |
| `F-171` | `F-policy-composition` |
| `F-172` | `F-tool-set-intersection` |
| `F-173` | `F-inheritance-rules` |
| `F-174` | `F-four-subagent-limits` |
| `F-175` | `F-cycle-guard` |
| `F-176` | `F-salvage-and-retry-cap` |
| `F-177` | `F-agent-events` |
| `F-178` | `F-agent-definitions` |
| `F-180` | `F-mcp-client` |
| `F-181` | `F-tool-name-namespacing` |
| `F-182` | `F-schema-cache` |
| `F-183` | `F-server-trust-policy` |
| `F-184` | `F-server-limits` |
| `F-185` | `F-server-output-sanitation` |
| `F-186` | `F-http-and-sse-transports` |
| `F-20` | `F-tool-trait` |
| `F-21` | `F-file-read-tool` |
| `F-22` | `F-file-write-tool` |
| `F-23` | `F-file-edit-tool` |
| `F-24` | `F-directory-list-tool` |
| `F-25` | `F-glob-tool` |
| `F-26` | `F-grep-tool` |
| `F-27` | `F-bash-tool` |
| `F-28` | `F-path-confinement` |
| `F-29` | `F-tool-approval-gate` |
| `F-30` | `F-todo-tool` |
| `F-31` | `F-bash-os-sandbox` |
| `F-32` | `F-ask-approval-policy` |
| `F-33` | `F-approval-mode-resolution` |
| `F-34` | `F-permission-over-acp` |
| `F-35` | `F-tui-approval-prompt` |
| `F-40` | `F-hook-trait-tier-1` |
| `F-41` | `F-lifecycle-hook-points` |
| `F-42` | `F-out-of-process-plugin-tier-2` |
| `F-43` | `F-plugin-schema-cache` |
| `F-44` | `F-slash-commands` |
| `F-45` | `F-skills-filesystem` |
| `F-46` | `F-prompt-templates` |
| `F-50` | `F-append-only-session-log` |
| `F-51` | `F-session-resume` |
| `F-52` | `F-session-branching` |
| `F-53` | `F-ephemeral-mode` |
| `F-54` | `F-pi-session-import` |
| `F-55` | `F-session-close` |
| `F-56` | `F-session-cancel-without-close` |
| `F-57` | `F-session-list` |
| `F-58` | `F-session-delete` |
| `F-59` | `F-session-fork` |
| `F-60` | `F-stable-prefix-for-kv-cache` |
| `F-61` | `F-full-tool-list-at-turn-one` |
| `F-62` | `F-context-compaction` |
| `F-63` | `F-branch-summary` |
| `F-64` | `F-short-system-prompt` |
| `F-65` | `F-context-hook` |
| `F-70` | `F-layered-config` |
| `F-71` | `F-environment-variable-override` |
| `F-72` | `F-credential-resolution` |
| `F-73` | `F-profile-support` |
| `F-80` | `F-minimal-tui` |
| `F-81` | `F-pure-function-render` |
| `F-82` | `F-non-blocking-input` |
| `F-83` | `F-themes` |
| `F-84` | `F-keybindings` |
| `F-85` | `F-custom-tool-renderer` |
| `F-86` | `F-time-to-first-frame` |
| `F-90` | `F-acp-frontend` |
| `F-91` | `F-prompt-command` |
| `F-92` | `F-steer-command` |
| `F-93` | `F-abort-command` |
| `F-94` | `F-session-commands-over-acp` |
| `F-95` | `F-extension-ui-sub-protocol` |

## Two old ids point at one reference

The catalogue listed two features twice, and the slug rewrite made the duplication
visible. `F-161` restated `F-41`, and `F-163` restated `F-44`. Each pair is one feature,
so each pair now shares one row and one slug.

