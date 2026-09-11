#!/usr/bin/env bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"
source "$SCRIPT_DIR/lib/m0-demo.sh"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

M0_DEMO_EXISTING_SERVICES=false
case "${1:-}" in
    --existing-services) M0_DEMO_EXISTING_SERVICES=true; shift ;;
    --help|-h) printf 'Usage: bash scripts/demo-m0.sh [--existing-services]\n'; exit 0 ;;
    '') ;;
    *) fail 'unknown M0 demo argument' ;;
esac
(($# == 0)) || fail 'unexpected extra M0 demo argument'

readonly API_WAIT_ATTEMPTS=100
readonly API_WAIT_SECONDS=0.1

readonly M0_DEMO_LAUNCHER_PID=$BASHPID
M0_DEMO_ROOT=""
M0_DEMO_CLEANUP_DONE=0
M0_DEMO_CLEANUP_STATUS=0
M0_DEMO_CORE_PID=""
M0_DEMO_SUPERVISOR_PID=""

resolve_target_dir() {
    if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
        printf '%s/target\n' "$REPO_ROOT"
    elif [[ "$CARGO_TARGET_DIR" = /* ]]; then
        printf '%s\n' "$CARGO_TARGET_DIR"
    else
        printf '%s/%s\n' "$REPO_ROOT" "$CARGO_TARGET_DIR"
    fi
}

cleanup() {
    local status=$?
    trap - EXIT
    trap '' INT TERM
    if ! m0_demo_cleanup; then
        [[ "$status" != 0 ]] || status=1
    fi
    exit "$status"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

wait_for_api() {
    local attempt deadline=$((SECONDS + 10))

    for ((attempt = 1; attempt <= API_WAIT_ATTEMPTS && SECONDS < deadline; attempt += 1)); do
        if [[ -S "$api_socket" ]] && \
            timeout --kill-after=1 2 "$forge_cli_bin" --socket "$api_socket" \
                get /v1/health >/dev/null 2>&1; then
            return 0
        fi
        if ! m0_demo_process_token "$M0_DEMO_CORE_PID" >/dev/null; then
            fail "Core stopped before its local API became ready"
        fi
        sleep "$API_WAIT_SECONDS"
    done

    fail "Core local API did not become ready within $((API_WAIT_ATTEMPTS / 10)) seconds"
}

if [[ $M0_DEMO_EXISTING_SERVICES != true ]]; then "$SCRIPT_DIR/dev-up.sh"; fi
require_dev_environment
wait_for_postgres
wait_for_nats
for executable in timeout od tr; do require_command "$executable"; done
cd "$REPO_ROOT"

# Build once before starting long-running processes. Polling through `cargo run`
# can otherwise wait on Cargo's workspace lock instead of testing the API.
cargo build --quiet --locked --bins --package forge-core \
    --package forge-supervisor --package forge-cli \
    --package forge-storage
readonly target_dir="$(resolve_target_dir)"
readonly forge_core_bin="$target_dir/debug/forge-core"
readonly forge_supervisor_bin="$target_dir/debug/forge-supervisor"
readonly forge_cli_bin="$target_dir/debug/forge-cli"
readonly forge_migrate_bin="$target_dir/debug/forge-migrate"
[[ -x "$forge_core_bin" && -x "$forge_supervisor_bin" && -x "$forge_cli_bin" \
    && -x "$forge_migrate_bin" ]] || \
    fail "Forge native binaries were not produced by cargo build"

M0_DEMO_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/forge-m0-runtime.XXXXXX")"
chmod 700 "$M0_DEMO_ROOT"
# Explicit daemon paths leave Podman's host XDG runtime directory unchanged.
readonly api_socket="$M0_DEMO_ROOT/api.sock"
readonly supervisor_socket="$M0_DEMO_ROOT/core.sock"
demo_nonce="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
readonly demo_nonce
M0_DEMO_DATABASE="forge_m0_$demo_nonce"
database_url="$(m0_demo_database_url "$M0_DEMO_DATABASE")"
readonly database_url
postgres="$(service_container_id postgres)"
readonly postgres
timeout --kill-after=1 15 podman exec "$postgres" createdb \
    --username "$FORGE_POSTGRES_USER" -- "$M0_DEMO_DATABASE" \
    >"$M0_DEMO_ROOT/database.log" 2>&1 \
    || fail 'dedicated M0 database creation failed; see retained database.log'
# Invoke the binary directly: migrate.sh reloads the shared development URL.
FORGE_DATABASE_URL="$database_url" timeout --kill-after=1 60 \
    "$forge_migrate_bin" >"$M0_DEMO_ROOT/migrate.log" 2>&1 \
    || fail 'dedicated M0 database migration failed'

# The URL remains in the inherited environment rather than a process argument.
FORGE_DATABASE_URL="$database_url" "$forge_core_bin" --fake-runtime \
    --api-socket "$api_socket" --supervisor-socket "$supervisor_socket" \
    --observability-address 127.0.0.1:0 >"$M0_DEMO_ROOT/core.log" 2>&1 &
M0_DEMO_CORE_PID=$!
M0_DEMO_CORE_TOKEN="$(m0_demo_process_token "$M0_DEMO_CORE_PID")" || M0_DEMO_CORE_TOKEN=''
wait_for_api

"$forge_supervisor_bin" \
    --socket "$supervisor_socket" \
    --state-directory "$M0_DEMO_ROOT/supervisor-state" --host-id "m0-demo-$demo_nonce" \
    --observability-address 127.0.0.1:0 >"$M0_DEMO_ROOT/supervisor.log" 2>&1 &
M0_DEMO_SUPERVISOR_PID=$!
M0_DEMO_SUPERVISOR_TOKEN="$(m0_demo_process_token "$M0_DEMO_SUPERVISOR_PID")" \
    || fail 'M0 Supervisor exited during startup; see retained supervisor.log'

timeout --kill-after=1 45 "$forge_cli_bin" --socket "$api_socket" demo m0
