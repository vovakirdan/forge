#!/usr/bin/env bash
# Offline, keyless version probes inside Podman. No host credential mounts.
set -euo pipefail
readonly IMAGE_REF="${1:-localhost/forge-runtime:m1}"
if [[ "$IMAGE_REF" == -* ]]; then
    printf 'Image reference must not be an option.\n' >&2
    exit 1
fi
probe() {
    podman run --rm --pull=never --network=none --http-proxy=false --read-only --cap-drop=all \
        --security-opt=no-new-privileges --userns=keep-id \
        --timeout 15 --pids-limit 64 --memory 536870912 --memory-swap 536870912 --cpus 1 \
        --user "$(id -u):$(id -g)" \
        --tmpfs /tmp:rw,nosuid,nodev,size=67108864 \
        --env HOME=/tmp --env XDG_CONFIG_HOME=/tmp/config \
        --env XDG_CACHE_HOME=/tmp/cache --env XDG_DATA_HOME=/tmp/data \
        --env OPENCODE_DISABLE_MODELS_FETCH=true --env OPENCODE_DISABLE_AUTOUPDATE=true \
        --env CLAUDE_CONFIG_DIR=/tmp/claude --env CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
        --entrypoint "$1" "$IMAGE_REF" --version
}
[[ $(probe /usr/local/bin/node) == v24.14.0 ]] || { printf 'Node version mismatch.\n' >&2; exit 1; }
[[ $(probe /usr/local/bin/codex) == 'codex-cli 0.153.2' ]] || { printf 'Codex version mismatch.\n' >&2; exit 1; }
[[ $(probe /usr/local/bin/opencode) == 1.18.29 ]] || { printf 'OpenCode version mismatch.\n' >&2; exit 1; }
[[ $(probe /usr/local/bin/claude) == '2.1.263 (Claude Code)' ]] || { printf 'Claude Code version mismatch.\n' >&2; exit 1; }
readonly DIGEST="$(podman image inspect --format '{{.Digest}}' "$IMAGE_REF")"
if [[ ! "$DIGEST" =~ ^sha256:[a-f0-9]{64}$ ]]; then
    printf 'Image digest unavailable.\n' >&2
    exit 1
fi
printf 'Keyless runtime version probes passed.\n'
printf 'RunSpec image: localhost/forge-runtime@%s\n' "$DIGEST"
