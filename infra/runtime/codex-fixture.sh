#!/bin/sh
# TASK-12 synthetic fixture only. It advertises the pinned wire version so
# sandbox tests exercise preflight; it does not certify a real provider runtime.
set -eu
if [ "${1:-}" = "--version" ]; then
    printf 'codex-cli 0.153.2\n'
    exit 0
fi
input=$(cat)
hold=${FORGE_FIXTURE_HOLD:-0}
# Explicit whole-line marker used only by the keyless Core-to-Podman acceptance fixture.
while IFS= read -r line; do
    [ "$line" != FORGE_ACCEPTANCE_HOLD_FOR_STOP ] || hold=1
done <<EOF
$input
EOF
printf '{"type":"turn.started"}\n'
printf '%s\n' "$input" > fixture-result.txt
if [ "$hold" = 1 ]; then
    trap 'exit 130' INT TERM
    while :; do sleep 1; done
fi
printf '{"type":"item.completed","item":{"type":"reasoning","text":"fixture private reasoning"}}\n'
printf '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}\n'
