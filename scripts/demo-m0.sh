#!/usr/bin/env bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

readonly API_WAIT_ATTEMPTS=100
readonly API_WAIT_SECONDS=0.1

runtime_dir=""
core_pid=""
supervisor_pid=""

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
    local pid

    trap - EXIT
    for pid in "$supervisor_pid" "$core_pid"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            kill -TERM "$pid" 2>/dev/null || true
        fi
    done
    for pid in "$supervisor_pid" "$core_pid"; do
        if [[ -n "$pid" ]]; then
            wait "$pid" 2>/dev/null || true
        fi
    done
    if [[ -n "$runtime_dir" && -d "$runtime_dir" && ! -L "$runtime_dir" ]]; then
        rm -rf -- "$runtime_dir"
    fi
    exit "$status"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

wait_for_api() {
    local attempt

    for ((attempt = 1; attempt <= API_WAIT_ATTEMPTS; attempt += 1)); do
        if [[ -S "$api_socket" ]] && \
            "$forge_cli_bin" --socket "$api_socket" get /v1/health >/dev/null 2>&1; then
            return 0
        fi
        if ! kill -0 "$core_pid" 2>/dev/null; then
            fail "Core stopped before its local API became ready"
        fi
        sleep "$API_WAIT_SECONDS"
    done

    fail "Core local API did not become ready within $((API_WAIT_ATTEMPTS / 10)) seconds"
}

"$SCRIPT_DIR/dev-up.sh"
require_dev_environment
"$SCRIPT_DIR/migrate.sh"
cd "$REPO_ROOT"

# Build once before starting long-running processes. Polling through `cargo run`
# can otherwise wait on Cargo's workspace lock instead of testing the API.
cargo build --quiet --locked --package forge-core --bin forge-core \
    --package forge-supervisor --package forge-cli
readonly target_dir="$(resolve_target_dir)"
readonly forge_core_bin="$target_dir/debug/forge-core"
readonly forge_supervisor_bin="$target_dir/debug/forge-supervisor"
readonly forge_cli_bin="$target_dir/debug/forge-cli"
[[ -x "$forge_core_bin" && -x "$forge_supervisor_bin" && -x "$forge_cli_bin" ]] || \
    fail "Forge native binaries were not produced by cargo build"

runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/forge-m0-runtime.XXXXXX")"
chmod 700 "$runtime_dir"
export XDG_RUNTIME_DIR="$runtime_dir"
readonly api_socket="$XDG_RUNTIME_DIR/forge/api.sock"
readonly supervisor_socket="$XDG_RUNTIME_DIR/forge/core.sock"

# The URL remains in the inherited environment rather than a process argument.
"$forge_core_bin" --fake-runtime >"$runtime_dir/core.log" 2>&1 &
core_pid=$!
wait_for_api

"$forge_supervisor_bin" \
    --socket "$supervisor_socket" \
    >"$runtime_dir/supervisor.log" 2>&1 &
supervisor_pid=$!

"$forge_cli_bin" --socket "$api_socket" demo m0
