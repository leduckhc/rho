#!/usr/bin/env bash
set -euo pipefail
cd /Users/le/Work/Vibe/rho-reasoning
exec codex exec -s read-only "$(cat /Users/le/Work/Vibe/rho-reasoning/.piano/codex-jobs/20260822-102444-reasoning-review/prompt.md)"
