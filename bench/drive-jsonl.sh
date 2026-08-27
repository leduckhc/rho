#!/usr/bin/env bash
# A JSONL client for rho, in plain shell.
#
# It writes one command per line to rho's stdin and reads one line at a time from rho's
# stdout, through two named pipes. It uses no bash feature newer than version 3.2, which
# is what macOS ships, so it runs anywhere. It is deliberately plain: if this works, any
# language works, which is the whole claim of the JSONL frontend.
#
# Usage: drive-jsonl.sh <case> [provider] [model]
#
# Cases:
#   prompt   a prompt, then wait for settled
#   steer    a prompt, a steer during the run, then wait for the delivery and settled
#   abort    a prompt, an abort, then check the run settles as cancelled
#   dialog   a prompt that needs tool approval, answered by nobody, so the timeout decides
#   dialog-yes  the same approval gate, answered yes, so the tool really runs
#   errors   an unknown command, a malformed line, and an unknown provider
#   twice    the same prompt twice down one session
#
# AWAIT_TIMEOUT bounds one read, in seconds. It defaults to 120, which clears a slow
# model and an approval dialog's own 30 second timeout.
set -uo pipefail

CASE="${1:?a case name}"
PROVIDER="${2:-bedrock}"
MODEL="${3:-global.anthropic.claude-haiku-4-5-20251001-v1:0}"
RHO="${RHO:-./target/release/rho}"

if [ "$CASE" = "dialog" ] || [ "$CASE" = "dialog-yes" ]; then
  export RHO_APPROVAL=ask
fi

echo "=== case: $CASE   provider: $PROVIDER   model: $MODEL"
echo "=== approval: ${RHO_APPROVAL:-allow-all (default)}"
echo "=== binary: $RHO"

WORK=$(mktemp -d /tmp/rho-jsonl.XXXXXX)
trap 'rm -rf "$WORK"' EXIT
mkfifo "$WORK/in" "$WORK/out"

"$RHO" jsonl --provider "$PROVIDER" --model "$MODEL" \
  <"$WORK/in" >"$WORK/out" 2>"$WORK/err.log" &
RHO_PID=$!

# Hold both pipes open from this shell. Opening the write end after rho starts reading
# is what keeps rho from seeing end of file immediately.
exec 3>"$WORK/in"
exec 4<"$WORK/out"

send() {
  printf '%s\n' "$1" >&3
  echo "--> $1"
}

# Read lines until one contains the pattern.
#
# Two bounds, because they catch different failures. The line budget catches a flood of
# the wrong lines. The read timeout catches silence: without it a hung rho blocked this
# script for ever, the budget never counted down, and the outer harness had to kill it.
# A hang must fail loudly, not look like a slow pass.
await() {
  pattern="$1"; budget="${2:-500}"
  LAST_LINE=""
  while [ "$budget" -gt 0 ]; do
    budget=$((budget - 1))
    if ! IFS= read -r -t "${AWAIT_TIMEOUT:-120}" line <&4; then
      echo "!!! rho wrote nothing for ${AWAIT_TIMEOUT:-120}s, or closed stdout, while waiting for: $pattern"
      return 1
    fi
    # Trim a very long delta so the transcript stays readable.
    if [ ${#line} -gt 200 ]; then echo "<-- ${line:0:200}..."; else echo "<-- $line"; fi
    LAST_LINE="$line"
    case "$line" in *"$pattern"*) return 0;; esac
  done
  echo "!!! line budget spent waiting for: $pattern"
  return 1
}

finish() {
  # Close rho's stdin. It drains any run to its settled event, then returns.
  exec 3>&-
  # Bound the wait. `await` grew a read timeout because a hung rho blocked this script
  # for ever, and an unbounded `wait` here reopens exactly that hole on the exit path.
  waited=0
  while kill -0 "$RHO_PID" 2>/dev/null; do
    if [ "$waited" -ge "${FINISH_TIMEOUT:-60}" ]; then
      echo "!!! rho did not exit within ${FINISH_TIMEOUT:-60}s after stdin closed. Killing it."
      kill -9 "$RHO_PID" 2>/dev/null
      rc=1
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$RHO_PID"
  code=$?
  echo "=== rho exit code: $code"
  echo "=== stderr:"
  sed 's/^/    /' "$WORK/err.log"
  return $code
}

rc=0
case "$CASE" in
  prompt)
    send '{"type":"prompt","req_id":"p1","message":"Say the single word: ready. Nothing else."}'
    await '"success":true' || rc=1
    await '"type":"settled"' || rc=1
    ;;

  steer)
    # A steer only lands at a turn boundary, so the run needs more than one turn. A tool
    # call gives it one: turn 1 calls the tool, turn 2 answers. The steer is queued
    # during turn 1 and must be delivered between the turns.
    #
    # A single-turn prompt is not a test of this. It queues the message and settles with
    # the message still queued, which is correct rho-core behaviour and proves nothing
    # about delivery.
    send '{"type":"prompt","req_id":"p1","message":"Use the bash tool to run: echo first. Then tell me what it printed."}'
    await '"success":true' || rc=1
    # Wait until a tool really starts, so the steer arrives while turn 1 is busy.
    await '"type":"tool_start"' || rc=1
    send '{"type":"steer","req_id":"s1","message":"When you answer, also say the word banana."}'
    await '"command":"steer"' || rc=1
    await '"type":"message_queued"' || rc=1
    # The proof: the queue drains at the boundary, before the next turn starts.
    await '"type":"message_delivered"' || rc=1
    await '"type":"settled"' || rc=1
    ;;

  abort)
    send '{"type":"prompt","req_id":"p1","message":"Write a very long essay about the sea. At least 800 words."}'
    await '"success":true' || rc=1
    # Let some text arrive, so the abort really lands mid-run.
    await '"type":"text_delta"' || rc=1
    send '{"type":"abort","req_id":"a1"}'
    # Wait for the settled event itself, and only then check the reason.
    #
    # Matching '"stop_reason":"cancelled"' alone was fragile: turn_end carries a
    # stop_reason field too, so the case could pass on a turn end and close stdin while
    # the run was still draining. It happened to work only because turn_end spells the
    # word with one l and settled spells it with two.
    await '"type":"settled"' || rc=1
    case "$LAST_LINE" in
      *'"type":"settled"'*'"stop_reason":"cancelled"'*)
        echo "    (settled as cancelled)" ;;
      *)
        echo "!!! the run settled, but not as cancelled: $LAST_LINE"
        rc=1 ;;
    esac
    ;;

  twice)
    send '{"type":"prompt","req_id":"p1","message":"Say only: one"}'
    await '"type":"settled"' || rc=1
    send '{"type":"prompt","req_id":"p2","message":"Say only: two"}'
    await '"success":true' || rc=1
    await '"type":"settled"' || rc=1
    send '{"type":"get_messages","req_id":"g1"}'
    await '"command":"get_messages"' || rc=1
    ;;

  dialog)
    # The session asks before a mutating tool, and this client answers nothing. The
    # agent-side timeout must resolve it, deny the tool, and let the run settle.
    #
    # RHO_APPROVAL=ask is what turns the gate on. Before this frontend, `ask` had no
    # answer in any headless build and rho refused to start with it.
    send '{"type":"prompt","req_id":"p1","message":"Create a file called note.txt containing the word hello. Use your tools."}'
    await '"success":true' || rc=1
    await '"type":"dialog"' || rc=1
    echo "    (answering nothing on purpose: the agent-side timeout must decide)"
    await '"type":"settled"' || rc=1
    ;;

  dialog-yes)
    # The same gate, answered. The client says yes with the matching id, and the tool
    # must then really run.
    send '{"type":"prompt","req_id":"p1","message":"Use the bash tool to run: echo approved"}'
    await '"success":true' || rc=1
    await '"type":"dialog"' || rc=1
    # Read the dialog id out of the last line seen. The id is minted by rho, so a client
    # must echo it back rather than invent one.
    DIALOG_ID=$(echo "$LAST_LINE" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
    echo "    (dialog id: $DIALOG_ID)"
    send "{\"type\":\"dialog_response\",\"id\":\"$DIALOG_ID\",\"answer\":{\"confirmed\":true}}"
    await '"type":"tool_end"' || rc=1
    await '"type":"settled"' || rc=1
    ;;

  errors)
    send '{"type":"compact","req_id":"e1"}'
    await '"error":"unknown_command"' || rc=1
    send '{not json'
    await '"error":"parse_error"' || rc=1
    send '{"type":"prompt","message":"hi","only_if_cheap":true}'
    await '"error":"parse_error"' || rc=1
    send '{"type":"set_model","req_id":"e4","provider":"nosuchprovider","model_id":"x"}'
    await '"error":"unknown_provider"' || rc=1
    # The same failure twice must behave the same way.
    send '{"type":"set_model","req_id":"e5","provider":"nosuchprovider","model_id":"x"}'
    await '"error":"unknown_provider"' || rc=1
    # And the session is still alive after every failure.
    send '{"type":"get_state","req_id":"e6"}'
    await '"command":"get_state"' || rc=1
    ;;

  *)
    echo "unknown case: $CASE"
    exit 2
    ;;
esac

finish || rc=$?
echo "=== case $CASE result: $([[ $rc -eq 0 ]] && echo PASS || echo FAIL)"
exit $rc
