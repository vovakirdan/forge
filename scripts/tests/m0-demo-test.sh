#!/usr/bin/env bash
# Synthetic boundary tests only. No services, real credentials or database I/O.
set -euo pipefail
umask 077
readonly TEST_SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$TEST_SCRIPT_DIR/../lib/m0-demo.sh"
readonly test_root="$(mktemp -d "${TMPDIR:-/tmp}/forge-m0-helper-test.XXXXXX")"

FORGE_POSTGRES_USER=forge
FORGE_POSTGRES_DATABASE=forge
FORGE_POSTGRES_PASSWORD=000000000000000000000000000000000000000000000000
FORGE_POSTGRES_HOST=127.0.0.1
FORGE_POSTGRES_PORT=55432
FORGE_DATABASE_URL="postgresql://forge:$FORGE_POSTGRES_PASSWORD@127.0.0.1:55432/forge"
readonly original_url="$FORGE_DATABASE_URL"
readonly database=forge_m0_00000000000000000000000000000001
result="$(m0_demo_database_url "$database")"
[[ "$result" == "${original_url%/*}/$database" ]]
[[ "$FORGE_DATABASE_URL" == "$original_url" ]]

refuses() {
    if m0_demo_database_url "$1" >"$test_root/output" 2>"$test_root/error"; then
        printf 'm0-demo helper accepted invalid scope\n' >&2
        exit 1
    fi
    [[ ! -s "$test_root/output" && -s "$test_root/error" ]]
    ! grep -Fq "$FORGE_POSTGRES_PASSWORD" "$test_root/error"
    ! grep -Fq 'postgresql://' "$test_root/error"
}

for invalid in forge public '--maintenance-db=forge' 'forge_m0_;DROP DATABASE forge' \
    forge_m0_0000000000000000000000000000000A \
    forge_m0_000000000000000000000000000000001 \
    "$database/other"; do
    refuses "$invalid"
done
for invalid in "$original_url?options=search_path%3Dpublic" \
    "$original_url#fragment" "$original_url/other" \
    "${original_url/127.0.0.1/foreign.example}" \
    "postgresql://forge:password/with/slashes@127.0.0.1:55432/forge"; do
    FORGE_DATABASE_URL="$invalid"
    refuses "$database"
done
FORGE_DATABASE_URL="$original_url"
FORGE_POSTGRES_PORT=65536
refuses "$database"
FORGE_POSTGRES_PORT=55432
FORGE_POSTGRES_HOST=foreign.example
refuses "$database"
FORGE_POSTGRES_HOST=127.0.0.1

# A mismatched child token must not signal even a genuine local child.
M0_DEMO_LAUNCHER_PID=$BASHPID
sleep 30 &
test_pid=$!
test_token="$(m0_demo_process_token "$test_pid")"
if m0_demo_stop_process "$test_pid" invalid-token 0 >/dev/null 2>&1; then
    printf 'm0-demo cleanup accepted a different process lifetime\n' >&2
    exit 1
fi
kill -0 "$test_pid"
# Zero grace intentionally forces just this owned synthetic child and reports it.
if m0_demo_stop_process "$test_pid" "$test_token" 0 >/dev/null 2>&1; then
    printf 'm0-demo cleanup concealed forced termination\n' >&2
    exit 1
fi
! kill -0 "$test_pid" 2>/dev/null

# Cleanup has no database/container/file deletion path and is parent-only/idempotent.
M0_DEMO_ROOT="$test_root/retained"
M0_DEMO_DATABASE="$database"
mkdir -m 700 "$M0_DEMO_ROOT"
printf 'retained evidence\n' >"$M0_DEMO_ROOT/core.log"
M0_DEMO_SUPERVISOR_PID=fixture-supervisor
M0_DEMO_CORE_PID=fixture-core
m0_demo_stop_process() { printf '%s\n' "$1" >>"$test_root/cleanup-order"; }
(m0_demo_cleanup)
[[ ! -e "$test_root/cleanup-order" ]]
m0_demo_cleanup 2>"$test_root/cleanup-output"
m0_demo_cleanup 2>>"$test_root/cleanup-output"
[[ "$(<"$test_root/cleanup-order")" == $'fixture-supervisor\nfixture-core' ]]
[[ "$(<"$M0_DEMO_ROOT/core.log")" == 'retained evidence' ]]
! grep -Fq "$FORGE_POSTGRES_PASSWORD" "$test_root/cleanup-output"
! grep -Fq 'postgresql://' "$test_root/cleanup-output"
printf 'm0-demo helper tests passed; retained synthetic fixtures: %s\n' "$test_root"
