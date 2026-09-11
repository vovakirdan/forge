#!/usr/bin/env bash
# Synthetic public projections and a controlled clock; no services or real waits.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$TEST_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/lib/m2-common.sh"
source "$SCRIPT_DIR/lib/m2-workflow.sh"

TEST_ROOT=$(mktemp -d /tmp/forge-m2-progress-test.XXXXXXXX)
test_fail() { printf 'm2-progress-test: %s\nRetained: %s\n' "$1" "$TEST_ROOT" >&2; exit 1; }
TASKS='{"items":[{"id":"a","key":"TASK-001","lifecycle":"done","current_stage_id":"integrate"},{"id":"b","key":"TASK-002","lifecycle":"in_progress","current_stage_id":"work","description":"SYNTHETIC_PRIVATE_TASK"}]}'
STOPPED='{"items":[{"id":"old-a","task_id":"a","stage_id":"work","observed_state":"stopped","desired_state":"stopped"},{"id":"old-b","task_id":"b","stage_id":"work","observed_state":"stopped","desired_state":"stopped"}]}'
run_fixture() {
    jq -cn --arg observed "$1" --arg desired "$2" --argjson prior "$STOPPED" '
        $prior | .items += [{id:"new-b",task_id:"b",stage_id:"work",observed_state:$observed,
            desired_state:$desired,prompt:"SYNTHETIC_PRIVATE_RUN"}]'
}
assert_progress() {
    local name=$1 runs=$2 expected=$3 awaiting=$4
    m2_progress "$TASKS" "$runs" 75 >"$TEST_ROOT/$name.log"
    rg -Fq "$expected" "$TEST_ROOT/$name.log" || test_fail "$name missed observed state"
    rg -Fq "ожидают исполнения=$awaiting" "$TEST_ROOT/$name.log" || test_fail "$name guessed Task scheduling state"
    rg -Fq 'до общего таймаута 75s' "$TEST_ROOT/$name.log" || test_fail "$name missed remaining time"
    ! rg -q 'SYNTHETIC_PRIVATE' "$TEST_ROOT/$name.log" || test_fail "$name leaked a payload"
    [[ $(wc -c <"$TEST_ROOT/$name.log") -lt 1000 ]] || test_fail "$name output is not bounded"
}

assert_progress no-runs '{"items":[]}' 'running=0' 1
assert_progress stopped "$STOPPED" 'running=0' 1
assert_progress pending "$(run_fixture unknown provision_requested)" 'ожидают запуска=1' 0
assert_progress provisioning "$(run_fixture provisioning provision_requested)" 'provisioning=1' 0
assert_progress running "$(run_fixture running running)" 'running=1' 0
assert_progress stopping "$(run_fixture stopping stop_requested)" 'stopping=1' 0
assert_progress unknown "$(run_fixture unknown running)" 'неопределённых=1' 0
assert_progress lost "$(run_fixture lost running)" 'неопределённых=1' 0
assert_progress failed "$(run_fixture failed failed)" 'running=0' 1
assert_progress unsafe "$(run_fixture 'SYNTHETIC_PRIVATE_STATE' running)" 'неопределённых=1' 0
COMMUNICATION=$(run_fixture running running | jq -c '.items[-1].task_id=null | .items[-1].stage_id=null')
assert_progress communication "$COMMUNICATION" 'running=1' 1
SYSTEM_TASKS=$(jq -c '.items[1].current_stage_id="integrate"' <<<"$TASKS")
m2_progress "$SYSTEM_TASKS" "$STOPPED" 10 >"$TEST_ROOT/system.log"
rg -Fq 'ожидают исполнения=0; на системной стадии=1' "$TEST_ROOT/system.log" \
    || test_fail 'system integration was misrepresented as a missing Employee Run'
m2_progress "$TASKS" "$STOPPED" -5 >"$TEST_ROOT/expired.log"
rg -Fq 'до общего таймаута 0s' "$TEST_ROOT/expired.log" || test_fail 'negative remaining time'

# Unset Bash special-clock behavior before assigning synthetic elapsed seconds.
unset SECONDS
SECONDS=100
M2_PROJECT_ID=project M2_RUN_GUARD=12 M1_CORE_PID=100 M1_SUPERVISOR_PID=101
M2_DEADLINE=190 TEST_DONE_AT=160
SLEEPS=0
m1_process_token() { [[ $1 == 100 || $1 == 101 ]]; }
m2_command() { test_fail 'progress reporting issued a management command'; }
m2_read() {
    [[ $1 == "/v1/projects/$M2_PROJECT_ID" ]] || test_fail 'unexpected detail route'
    printf '%s\n' '{"execution_gate":"open"}'
}
m2_pages() {
    case "$1" in
        "/v1/projects/$M2_PROJECT_ID/runs") printf '%s\n' "$STOPPED" ;;
        "/v1/projects/$M2_PROJECT_ID/tasks")
            if ((SECONDS >= TEST_DONE_AT)); then jq -c '.items[1].lifecycle="done"' <<<"$TASKS"
            else printf '%s\n' "$TASKS"; fi ;;
        *) test_fail 'unexpected list route' ;;
    esac
}
sleep() {
    [[ $1 == 1 ]] || test_fail 'unexpected polling delay'
    SECONDS=$((SECONDS+15))
    SLEEPS=$((SLEEPS+1))
    ((SLEEPS < 10)) || test_fail 'workflow did not terminate'
}
m2_wait_done >"$TEST_ROOT/cadence.log" || test_fail 'synthetic completion failed'
[[ $SLEEPS == 4 && $M2_DEADLINE == 190 ]] || test_fail 'polling or deadline changed'
[[ $(rg -c '^M2:' "$TEST_ROOT/cadence.log") == 2 ]] || test_fail 'progress cadence is not 30 seconds'
rg -Fq 'до общего таймаута 90s' "$TEST_ROOT/cadence.log" || test_fail 'initial progress missing'
rg -Fq 'до общего таймаута 60s' "$TEST_ROOT/cadence.log" || test_fail 'unchanged-stage progress missing'

SECONDS=100 M2_DEADLINE=125 TEST_DONE_AT=1000 SLEEPS=0
if m2_wait_done >"$TEST_ROOT/timeout.log" 2>&1; then test_fail 'progress defeated session timeout'; fi
[[ $SLEEPS == 2 && $M2_DEADLINE == 125 ]] || test_fail 'timeout was reset or delayed'
rg -Fq 'Истёк общий срок сценария.' "$TEST_ROOT/timeout.log" || test_fail 'timeout reason changed'
! rg -q 'SYNTHETIC_PRIVATE' "$TEST_ROOT/cadence.log" || test_fail 'polling leaked private Task data'
printf 'm2-progress-test: passed (synthetic state counts, 30-second cadence and unchanged deadline); retained %s\n' "$TEST_ROOT"
