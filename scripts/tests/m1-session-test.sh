#!/usr/bin/env bash

# Synthetic lifecycle/ownership tests. No daemon, database or provider is used.
set -euo pipefail
umask 077
readonly test_root="$(mktemp -d "${TMPDIR:-/tmp}/forge-m1-session-test.XXXXXXXX")"
readonly source_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
source "$source_root/scripts/lib/m1-session.sh"
M1_ROOT="$test_root/session"
M1_EXEC="$M1_ROOT/exec"
M1_HOST=01900000-0000-7000-8000-000000000099
readonly run_id=01900000-0000-7000-8000-000000000001
printf -v owned_id '%064x' 1
printf -v foreign_id '%064x' 2
mkdir -p "$M1_ROOT"

expect_status() {
    local expected="$1" actual=0
    shift
    "$@" || actual=$?
    if [[ "$actual" != "$expected" ]]; then
        printf 'expected status %s, got %s: %s\n' "$expected" "$actual" "$*" >&2
        exit 1
    fi
}

fixture() {
    local id="$1" source="$2" destination="${3:-/run/forge}" host="${4:-$M1_HOST}" managed="${5:-true}"
    jq -n --arg source "$source" --arg destination "$destination" --arg host "$host" --arg managed "$managed" \
        '{labels:{"forge.host":$host,"forge.managed":$managed},mounts:[{Source:$source,Destination:$destination}],running:true}' \
        >"$test_root/$id.json"
}

# Persistent fixture files make effects observable across command substitutions.
m1_podman() {
    printf '%s\n' "$*" >>"$test_root/podman.calls"
    case "$1" in
        ps) [[ "${listing_fails:-0}" == 0 ]] || return 1; printf '%s\n' "$listing" ;;
        inspect) [[ "${inspect_fails:-0}" == 0 ]] || return 1; jq . "$test_root/${@: -1}.json" ;;
        stop|kill)
            [[ "${stop_fails:-0}" == 0 ]] || return 1
            jq '.running=false' "$test_root/${@: -1}.json" >"$test_root/changed.json"
            mv "$test_root/changed.json" "$test_root/${@: -1}.json"
            ;;
        *) return 99 ;;
    esac
}

fixture "$owned_id" "$M1_EXEC/runtime/$run_id/1"
listing="$owned_id"
[[ "$(m1_container_state "$owned_id")" == running ]]
expect_status 2 m1_scan_owned check
[[ "$(jq -r .running "$test_root/$owned_id.json")" == true ]]
expect_status 2 m1_scan_owned stop
expect_status 0 m1_scan_owned check
[[ -f "$test_root/$owned_id.json" ]]

for foreign_mount in \
    "$M1_EXEC-extra/runtime/$run_id/1" \
    "$M1_EXEC/runtime/not-a-run/1" \
    "$M1_EXEC/runtime/$run_id/0" \
    "$M1_EXEC/runtime/$run_id/-1" \
    "$M1_EXEC/runtime/$run_id/18446744073709551616" \
    "$M1_EXEC/runtime/$run_id/1/child" \
    "$M1_EXEC/runtime/$run_id/../1"; do
    fixture "$foreign_id" "$foreign_mount"
    [[ "$(m1_container_state "$foreign_id")" == foreign ]]
done
fixture "$foreign_id" "$M1_EXEC/runtime/$run_id/1" /another/mount
[[ "$(m1_container_state "$foreign_id")" == foreign ]]
fixture "$foreign_id" "$M1_EXEC/runtime/$run_id/1" /run/forge another-host
[[ "$(m1_container_state "$foreign_id")" == foreign ]]
fixture "$foreign_id" "$M1_EXEC/runtime/$run_id/1" /run/forge "$M1_HOST" false
listing="$foreign_id"
expect_status 0 m1_scan_owned stop
[[ "$(jq -r .running "$test_root/$foreign_id.json")" == true ]]

# Invalid IDs are never inspected/stopped; failures cannot mean quiet.
listing='--all'
expect_status 1 m1_scan_owned stop
listing="$owned_id"
listing_fails=1
expect_status 1 m1_scan_owned stop
listing_fails=0 inspect_fails=1
expect_status 1 m1_scan_owned stop
inspect_fails=0 stop_fails=1
fixture "$owned_id" "$M1_EXEC/runtime/$run_id/1"
expect_status 1 m1_scan_owned stop
stop_fails=0
if rg -q '^(stop|kill).*--all|inspect.*--all|^(stop|kill).*0000000000000002$' "$test_root/podman.calls"; then
    printf 'an unowned container was targeted\n' >&2
    exit 1
fi

# Signal only the recorded direct-child lifetime; force is reported as failure.
sleep 30 &
M1_TEST_PID=$!
M1_TEST_TOKEN="$(m1_process_token "$M1_TEST_PID")"
saved_token="$M1_TEST_TOKEN"
M1_TEST_TOKEN=not-the-same-process
expect_status 1 m1_stop_process M1_TEST 0
kill -0 "$M1_TEST_PID"
M1_TEST_TOKEN="$saved_token"
expect_status 1 m1_stop_process M1_TEST 0
! kill -0 "$M1_TEST_PID" 2>/dev/null
bash -c 'exit 7' &
M1_TEST_PID=$!
sleep 0.05
expect_status 1 m1_stop_process M1_TEST
bash -c 'exit 0' &
M1_TEST_PID=$!
sleep 0.05
expect_status 0 m1_stop_process M1_TEST

# Track order without a real Supervisor: DB/API failure still gets both physical
# passes, after stopping the owner and before stopping Core; repeated EXIT is inert.
m1_stop_project() { printf 'project\n' >>"$test_root/cleanup.order"; return 1; }
m1_stop_process() { printf '%s\n' "$1" >>"$test_root/cleanup.order"; }
m1_scan_owned() { printf 'scan:%s\n' "$1" >>"$test_root/cleanup.order"; [[ "$1" == stop ]]; }
sleep() { printf 'settle\n' >>"$test_root/cleanup.order"; }
M1_PROJECT_ID=$run_id
expect_status 1 m1_cleanup
[[ "$(<"$test_root/cleanup.order")" == $'project\nscan:check\nM1_SUPERVISOR\nscan:stop\nsettle\nscan:stop\nM1_CORE' ]]
before="$(<"$test_root/cleanup.order")"
expect_status 1 m1_cleanup
[[ "$(<"$test_root/cleanup.order")" == "$before" ]]
M1_CLEANUP_DONE=0
(m1_cleanup)
[[ "$(<"$test_root/cleanup.order")" == "$before" ]]

# A container appearing while cancellation settles must affect the final status.
M1_PROJECT_ID=''
m1_scan_owned() { [[ -f "$test_root/first-scan" ]] && return 2; : >"$test_root/first-scan"; }
expect_status 1 m1_cleanup
printf 'm1-session tests passed; synthetic fixtures retained at %s\n' "$test_root"
