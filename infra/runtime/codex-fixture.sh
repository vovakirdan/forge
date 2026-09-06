#!/bin/sh
# TASK-12 synthetic fixture only. It advertises the pinned wire version so
# sandbox tests exercise preflight; it does not certify a real provider runtime.
set -eu
if [ "${1:-}" = "--version" ]; then
    printf 'codex-cli 0.153.2\n'
    exit 0
fi
input=$(cat)
printf '{"type":"turn.started"}\n'
printf '%s\n' "$input" > fixture-result.txt
if [ "${FORGE_FIXTURE_HOLD:-0}" = 1 ]; then
    trap 'exit 130' INT TERM
    while :; do sleep 1; done
fi
printf '{"type":"item.completed","item":{"type":"reasoning","text":"fixture private reasoning"}}\n'
printf '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}\n'
