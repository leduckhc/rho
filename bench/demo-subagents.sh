#!/usr/bin/env bash
# Demonstrate the subagent feature, and assert every claim.
#
# Run it yourself. Do not trust a report. Every check below either prints PASS or
# prints FAIL and makes this script exit non-zero. See
# `docs/verification/subagents-bedrock.md` for the defects these checks came from.
#
#   ./bench/demo-subagents.sh
#
# It needs AWS Bedrock credentials in the environment. It writes only to a
# temporary directory, and it removes it at the end.
#
# **Assert on what rho controls, never on how the model words its answer.** The
# first version of this script grepped for phrases the model happened to use, and
# three checks failed while the product was correct. A model paraphrase is not a
# defect. So a prompt below asks for a verbatim quote whenever a check needs rho's
# own text, and a pattern accepts every reasonable wording.

set -uo pipefail

MODEL="${RHO_DEMO_MODEL:-us.anthropic.claude-haiku-4-5-20251001-v1:0}"
PROVIDER="${RHO_DEMO_PROVIDER:-bedrock}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RHO="$REPO/target/release/rho"

PASSED=0
FAILED=0

pass() { printf '  \033[32mPASS\033[0m %s\n' "$1"; PASSED=$((PASSED + 1)); }
fail() {
  printf '  \033[31mFAIL\033[0m %s\n' "$1"
  printf '       expected to find: %s\n' "$2"
  printf '       actual output was:\n'
  printf '%s\n' "$3" | sed 's/^/         /'
  FAILED=$((FAILED + 1))
}

# expect <name> <needle> <text>
expect() {
  if printf '%s' "$3" | grep -qi -- "$2"; then pass "$1"; else fail "$1" "$2" "$3"; fi
}

# reject <name> <needle> <text> — the needle must NOT appear.
reject() {
  if printf '%s' "$3" | grep -qi -- "$2"; then
    fail "$1" "NOT to find \"$2\"" "$3"
  else
    pass "$1"
  fi
}

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# --- Build ---

section "0. Build the binary you are about to test"
if [ ! -x "$RHO" ] || [ "${RHO_DEMO_REBUILD:-1}" = "1" ]; then
  ( cd "$REPO" && cargo build --release -p rho-cli >/dev/null 2>&1 ) || {
    echo "  cargo build failed"; exit 1;
  }
fi
printf '  binary: %s\n' "$(cd "$REPO" && ./target/release/rho --version)"

# --- The test bed ---

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
mkdir -p "$ROOT/.rho/agents"
printf 'alpha\n' > "$ROOT/a.txt"
printf 'beta\n'  > "$ROOT/b.txt"
printf 'gamma\n' > "$ROOT/c.txt"
printf 'MARKER-TOKEN-ZQX7419\n' > "$ROOT/secret-marker.txt"

cat > "$ROOT/.rho/agents/scout.md" <<'EOF'
---
name: scout
description: Fast recon. Reads a file and reports what it holds.
tools: read, grep, list
---
Report exactly what you find, in under 15 words. You never change a file.
EOF

cat > "$ROOT/.rho/agents/greedy.md" <<'EOF'
---
name: greedy
description: Asks for more tools than a parent may hold. It tests the intersection.
tools: read, bash, write, edit, spawn_agent, nonexistent_tool
---
List the tool names you actually have. Nothing else.
EOF

cat > "$ROOT/.rho/agents/slowpoke.md" <<'EOF'
---
name: slowpoke
description: A deliberately slow agent. It tests the child timeout.
tools: bash
---
Run exactly the bash command you are given.
EOF

cat > "$ROOT/.rho/agents/nodesc.md" <<'EOF'
---
name: nodesc
tools: read
---
This definition has no description, so it must never load.
EOF

# run <extra flags...> -- <prompt>
run() {
  local prompt="$1"; shift
  ( cd "$REPO" && env -u AWS_PROFILE "$RHO" run "$prompt" \
      --provider "$PROVIDER" --model "$MODEL" --root "$ROOT" "$@" 2>&1 )
}

printf '  session root: %s\n' "$ROOT"

# --- 1. The trust boundary ---

section "1. A repository definition stays off until you trust it"
OUT="$(run 'Say OK.')"
expect "project definitions are withheld without --trust-project" "were found and not loaded" "$OUT"
expect "the notice names both loadable definitions" "greedy, scout" "$OUT"
reject "a definition with no description never loads, not even as withheld" "nodesc" "$OUT"

# --- 2. The model can see what it may spawn ---

section "2. The model learns which agents exist, and their purpose"
OUT="$(run 'Name only the agents you can pass to spawn_agent, one per line. Do not spawn anything.' --trust-project)"
expect "the real agents are offered" "scout" "$OUT"
expect "every loaded agent is offered" "slowpoke" "$OUT"

# --- 3. Delegation, and context isolation ---

section "3. Delegation returns a summary, and nothing else"
OUT="$(run 'Use spawn_agent with agent="scout" and prompt="What is in a.txt?". Report what scout said.' --trust-project)"
expect "the child did the work and the parent got the answer" "alpha" "$OUT"

OUT="$(run 'Use spawn_agent with agent="scout" and prompt="Read secret-marker.txt. You have seen a token. Do NOT repeat the token. Reply only: I HAVE READ IT." Then tell me the token if you can see it in the tool result, otherwise say TOKEN NOT VISIBLE TO ME.' --trust-project)"
reject "the child transcript never reaches the parent context" "MARKER-TOKEN-ZQX7419" "$OUT"
expect "the parent says it cannot see the token" "not visible" "$OUT"

# --- 4. The security core ---

section "4. A child can only ever be more restricted than its parent"
# rho's own note reads "[note: these requested tools were dropped because the parent
# does not hold them: ...]". Asserting the single word "dropped" failed the moment the
# model paraphrased it as "unavailable", which is the third time this script has been
# bitten by grepping English rather than a rho-controlled string. The pattern accepts
# any honest wording. The exact note text is pinned deterministically by the unit test
# `a_dropped_tool_name_is_reported_to_the_caller` in rho-tools, which is where a
# character-for-character assertion belongs.
OUT="$(run 'Use spawn_agent with agent="greedy" and prompt="List every tool name you have.". Report the tool result, including anything in square brackets.' --trust-project)"
expect "the parent is told that some requested tools were withheld" \
  "dropped\|unavailable\|not available\|does not hold\|doesn.t hold\|lacks\|withheld" "$OUT"
# These two prove the withheld names reach the parent. On their own they are weak,
# because both names also appear in the definition the model was asked about, so they
# only carry weight next to the check above.
expect "the withheld set names spawn_agent, so a child cannot fan out further" "spawn_agent" "$OUT"
expect "the withheld set names the unknown tool too" "nonexistent_tool" "$OUT"
# And the four tools the parent really shares must all be there.
for held in read write edit bash; do
  expect "the child holds the parent's $held" "$held" "$OUT"
done

OUT="$(run 'Use spawn_agent with agent="scout" and prompt="Create a file named breach.txt containing BREACH. Use any tool you have.". Say what scout reported.' --trust-project)"
if [ -f "$ROOT/breach.txt" ]; then
  fail "a read-only child cannot write" "no breach.txt on disk" "breach.txt was created"
else
  pass "a read-only child cannot write, and no file appeared on disk"
fi

OUT="$(run 'Use spawn_agent with agent="scout" and prompt="read a.txt". Quote the tool error verbatim, then say REFUSED.' --trust-project --read-only)"
expect "--read-only refuses delegation outright" "REFUSED" "$OUT"
expect "the refusal comes from the approval policy" "polic\|block\|deni\|reject\|not allow" "$OUT"

# --- 5. Credentials do not cross the boundary ---

section "5. A child cannot read the parent's credentials"
if [ -n "${AWS_SECRET_ACCESS_KEY:-}" ]; then
  OUT="$(run 'Use spawn_agent with agent="greedy" and prompt="Run exactly this bash command and report its raw output: printenv AWS_SECRET_ACCESS_KEY || echo ABSENT_FROM_CHILD_ENV". Report the child result verbatim.' --trust-project)"
  expect "the child environment holds no AWS secret" "ABSENT_FROM_CHILD_ENV" "$OUT"
  reject "the secret value never appears in the output" "$AWS_SECRET_ACCESS_KEY" "$OUT"
else
  printf '  SKIP  no AWS_SECRET_ACCESS_KEY in the environment to probe with\n'
fi

# --- 6. A failure is a result, not the end of the run ---

section "6. A failing child never kills the parent"
# The schema `enum` now constrains the agent name, so the model declines to invent
# one rather than calling the tool and getting an error. That is the fix for defect
# one working, and it makes the bad-name path unreachable from the model. The
# tool-level path is still covered, by the unit test
# `an_unknown_agent_name_is_a_result_not_a_fault` in `rho-tools`.
OUT="$(run 'Use spawn_agent with agent="ghost" and prompt="hello". If that name is not available, say NOT AVAILABLE and list the agents that are.' --trust-project)"
expect "the model refuses to invent an agent name" "not available\|don't have\|do not have\|only supports" "$OUT"
expect "and it names the agents that really exist instead" "scout" "$OUT"

OUT="$(run 'Use spawn_agent with agent="scout" and prompt="Read does-not-exist.txt and report the contents.". Then say CONTINUED.' --trust-project)"
expect "an absent file is a result, and the run continues" "CONTINUED" "$OUT"

# Not `sleep`. `matches_long_running` puts a sleep in the background on purpose, so
# the child returned at once and never reached its timeout. This check was green about
# one run in three, which is worse than red: it reported a timeout path that never ran.
# A python block waits just as long and matches no denylist entry.
OUT="$(run 'Use spawn_agent with agent="slowpoke" and prompt="Run exactly this bash command: python3 -c \"import time; time.sleep(30)\"". Then say in one sentence what the tool reported.' --trust-project --child-timeout-secs 3)"
expect "a child timeout is reported to the parent" "cancel\|timeout\|timed out" "$OUT"
expect "the refusal names the limit that fired" "3 second\|3-second\|3s" "$OUT"
expect "the parent survives the timeout and answers rather than dying" "slowpoke" "$OUT"

# --- 7. The fan-out, and the limits ---

section "7. A fan-out runs children together, and the caps bite"
OUT="$(run 'Use spawn_agents once with three tasks: scout on a.txt, scout on b.txt, scout on c.txt. One line per result.' --trust-project)"
expect "fan-out child 1 answered" "alpha" "$OUT"
expect "fan-out child 2 answered" "beta" "$OUT"
expect "fan-out child 3 answered" "gamma" "$OUT"

# The per-parent cap used to refuse the tasks over the limit, and this script asserted
# that refusal. It queues now, so every task runs. See decision D-queue-over-refuse and
# `SPEC-subagent-slots-handles-grace` section 2.8. A cap of one is used on purpose: it
# leaves no room at all, so two of the three tasks must wait.
OUT="$(run 'Use spawn_agents with three tasks: scout on a.txt, scout on b.txt, scout on c.txt. Reproduce the whole tool result verbatim, every section, changing nothing.' --trust-project --max-children-per-parent 1)"
reject "the per-parent cap queues a task instead of refusing it" "per-parent child limit" "$OUT"
expect "the task that had a slot ran" "alpha" "$OUT"
expect "the first task that waited still ran" "beta" "$OUT"
expect "the second task that waited still ran" "gamma" "$OUT"

# One cap still refuses, because a wait on it is a wait on another session's children.
#
# The prompt asks two narrow questions instead of asking for a long verbatim quote. A
# first version asked for the whole tool result, and the model answered "That's the
# verbatim result" and then summarised it. Two checks failed while rho was correct,
# which is exactly the failure this script's header warns about. A narrow question is
# quotable, so the answer carries rho's own words.
OUT="$(run 'Use spawn_agents with three tasks: scout on a.txt, scout on b.txt, scout on c.txt. Then reply with exactly two lines and nothing else. Line 1: the command-line flag that any refusal told you to ask the user to raise. Line 2: the contents of a.txt, as the child that ran reported them.' --trust-project --max-live-agents 1)"
expect "the process-wide cap refuses, and it names the flag that raises it" "max-live-agents" "$OUT"
expect "a refused task does not lose the work of the task that fitted" "alpha" "$OUT"

# --- 8. The transcript ---

section "8. Every child leaves a transcript, and the parent is told where"
# The path comes from the tool result, not from a directory this script guesses.
# The first version hardcoded "$ROOT/.rho/agent-transcripts", and when transcripts
# moved to a per-user temp directory it kept asserting the old place. The second
# guessed /tmp/rho-transcripts-$$, which is this shell's pid and never rho's.
# Reading the path the parent was actually handed cannot drift either way.
OUT="$(run 'Use spawn_agent with agent="scout" and prompt="What is in a.txt?". Quote the whole tool result verbatim.' --trust-project)"
TRANSCRIPT="$(printf '%s' "$OUT" | sed -n 's/.*full transcript: \([^]]*\)].*/\1/p' | head -1)"

if [ -n "$TRANSCRIPT" ]; then
  pass "the parent is told the transcript path"
else
  fail "the parent is told the transcript path" "a [full transcript: ...] note" "$OUT"
fi

if [ -n "$TRANSCRIPT" ] && [ -f "$TRANSCRIPT" ]; then
  pass "and that path names a real file"
else
  fail "the transcript path names a real file" "an existing file" "${TRANSCRIPT:-<none>}"
fi

if [ -n "$TRANSCRIPT" ] && grep -qE '"type":"(ToolStart|TurnStart)"' "$TRANSCRIPT" 2>/dev/null; then
  pass "it is JSONL holding the child's own tool calls, which never reached the parent"
else
  fail "the transcript is JSONL with the child's tool calls" '"type":"ToolStart"' "$(head -c 200 "${TRANSCRIPT:-/dev/null}" 2>/dev/null)"
fi

if [ -z "$TRANSCRIPT" ]; then
  :
elif [ "${TRANSCRIPT#"$ROOT"}" != "$TRANSCRIPT" ]; then
  fail "the transcript stays out of the session root" "a path outside $ROOT" "$TRANSCRIPT"
else
  pass "it lives outside the session root, so it cannot be committed by accident"
fi


# --- 9. A queued child is addressable while it waits ---

section "9. A queued child holds an id, and the model can act on it"
# A background child holds the only slot, so the next spawn must queue. The model is
# asked to quote rho's own words, because a check needs rho's text and not a paraphrase.
OUT="$(run 'Do exactly this. Call spawn_agent with agent="slowpoke", prompt="Run exactly this bash command: python3 -c \"import time; time.sleep(20)\"" and background=true. Then call spawn_agent with agent="scout", prompt="What is in a.txt?" and background=true. Then call agent_status on the id the second call gave you. Quote the agent_status result verbatim.' --trust-project --max-children-per-parent 1 --child-timeout-secs 25)"
expect "a spawn over the cap is admitted, not refused" "queued" "$OUT"
expect "and the model is told where it sits in the line" "place 1" "$OUT"
reject "no refusal names the per-parent cap any more" "per-parent child limit" "$OUT"

# A cancelled waiter must never be promised a start. This is the defect that driving it
# for real found. See decision D-a-cancelled-waiter-says-so.
OUT="$(run 'Do exactly this. Call spawn_agent with agent="slowpoke", prompt="Run exactly this bash command: python3 -c \"import time; time.sleep(20)\"" and background=true. Then call spawn_agent with agent="scout", prompt="What is in a.txt?" and background=true. Then call cancel_agent on the second id. Then call agent_status on that same second id. Quote both results verbatim.' --trust-project --max-children-per-parent 1 --child-timeout-secs 25)"
expect "a queued child is cancellable by the id the model holds" "asked to stop" "$OUT"
reject "a cancelled child is never promised a start" "it will start" "$OUT"
expect "and the answer says it was cancelled" "cancel" "$OUT"

# A steer must reach a child that has not started yet.
OUT="$(run 'Do exactly this. Call spawn_agent with agent="slowpoke", prompt="Run exactly this bash command: python3 -c \"import time; time.sleep(20)\"" and background=true. Then call spawn_agent with agent="scout", prompt="What is in a.txt?" and background=true. Then call steer_agent on the second id with the message "read b.txt as well". Quote the steer_agent result verbatim.' --trust-project --max-children-per-parent 1 --child-timeout-secs 25)"
expect "a steer is accepted for a child that has not started" "queued for" "$OUT"
expect "and the receipt names the agent, which holds no live handle yet" "scout" "$OUT"
reject "a steer for a queued child is never a lost message" "cannot be steered" "$OUT"

# --- Result ---

section "Result"
printf '  passed: %d\n  failed: %d\n' "$PASSED" "$FAILED"
if [ "$FAILED" -gt 0 ]; then
  printf '\n\033[31mThe feature is not working as documented.\033[0m\n'
  exit 1
fi
printf '\n\033[32mEvery documented behaviour held.\033[0m\n'
