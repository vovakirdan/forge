#!/usr/bin/env bash
# Explicit paid opt-in; never invoked by test-m2 or common integration.
set -euo pipefail
if [[ ${FORGE_M2_LIVE:-} != 1 ]]; then
    printf 'Not run: set FORGE_M2_LIVE=1 only after approving two real concurrent provider Runs.\n' >&2
    exit 1
fi
if [[ ${FORGE_M2_LIVE_SETTINGS:-} != /* ]]; then
    printf 'Require explicit absolute FORGE_M2_LIVE_SETTINGS. No auth/model discovery.\n' >&2
    exit 1
fi
[[ -f $FORGE_M2_LIVE_SETTINGS && ! -L $FORGE_M2_LIVE_SETTINGS \
    && $(stat -c '%u:%a' "$FORGE_M2_LIVE_SETTINGS") == "$(id -u):600" ]] || {
    printf 'Settings must be an owner-only regular file (0600), not a symlink.\n' >&2
    exit 1
}
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
cd "$FORGE_ROOT_DIR"
require_command jq
require_command cargo
require_command podman
require_rootless_podman
image=$(jq -er '.image | select(type == "string")' "$FORGE_M2_LIVE_SETTINGS")
[[ $image =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/-]*@sha256:[a-f0-9]{64}$ ]] || fail 'explicit digest-pinned image required'
podman image exists "$image" || fail 'build and select the current runtime image first'
bash "$SCRIPT_DIR/check-runtime-image.sh" "$image"
require_dev_environment
wait_for_postgres
wait_for_nats
printf 'REAL selected M2 lane: two concurrent Tasks, native follow-up, then managed stop.\n'
printf 'This consumes quota/credits. Run alone; do not interrupt cleanup. Private evidence is retained.\n'
FORGE_INTEGRATION=1 cargo test -p forge-testkit --test m2_provider_live --locked \
    live_m2_selected_provider_preserves_session_input_and_isolation \
    -- --ignored --exact --test-threads=1 --nocapture
