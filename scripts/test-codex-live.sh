#!/usr/bin/env bash
# REAL subscription usage: do not call from unattended/synthetic integration.
set -euo pipefail

# Validate opt-in before sourcing any local configuration or inspecting auth.
if [[ ${FORGE_CODEX_LIVE:-} != 1 ]]; then
    printf 'Not run: set FORGE_CODEX_LIVE=1 only after approving real subscription usage.\n' >&2
    exit 1
fi
if [[ ${FORGE_CODEX_AUTH_FILE:-} != /* || -z ${FORGE_CODEX_MODEL:-} \
    || ! ${FORGE_CODEX_RUNTIME_IMAGE:-} =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/-]*@sha256:[a-f0-9]{64}$ ]]; then
    printf 'Require explicit absolute FORGE_CODEX_AUTH_FILE, FORGE_CODEX_MODEL and digest-pinned FORGE_CODEX_RUNTIME_IMAGE. No defaults.\n' >&2
    exit 1
fi
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"
cd "$FORGE_ROOT_DIR"
require_command cargo
require_command podman
require_rootless_podman
podman image exists "$FORGE_CODEX_RUNTIME_IMAGE" || fail "build and explicitly select the current real runtime image first"
# Offline CLI/version check precedes any auth read; the Rust test enforces owner-only files.
bash "$SCRIPT_DIR/check-runtime-image.sh" "$FORGE_CODEX_RUNTIME_IMAGE"
require_dev_environment
wait_for_postgres
wait_for_nats
"$SCRIPT_DIR/migrate.sh"
printf 'REAL Codex subscription gate: two concurrent isolated Runs, then named stop. Usage may consume quota or credits.\n'
printf 'Run alone; do not interrupt cleanup. Private auth/evidence storage is retained as documented.\n'
FORGE_INTEGRATION=1 cargo test --package forge-testkit --test m1_codex_live --locked \
    live_codex_subscription_two_parallel_runs_isolated_stop_and_evidence \
    -- --ignored --exact --test-threads=1 --nocapture
