#!/usr/bin/env bash
# Initial READY health checks use synthetic public responses, never providers.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$TEST_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/lib/m2-common.sh"
source "$SCRIPT_DIR/lib/m2-workflow.sh"

TEST_ROOT=$(mktemp -d /tmp/forge-m2-ready-test.XXXXXXXX)
test_fail() { printf 'm2-ready-test: %s\nRetained: %s\n' "$1" "$TEST_ROOT" >&2; exit 1; }
M2_ROOT=$TEST_ROOT
M2_PROJECT_ID=01990000-0000-7000-8000-000000000001
M2_TASK_A=01990000-0000-7000-8000-000000000002
M2_TASK_B=01990000-0000-7000-8000-000000000003
M2_WRITER=01990000-0000-7000-8000-000000000004
RUN_A=01990000-0000-7000-8000-000000000005
RUN_B=01990000-0000-7000-8000-000000000006
M2_THREAD_A=a M2_THREAD_B=b M2_READY_A=ra M2_READY_B=rb
M1_CORE_PID=100 M1_SUPERVISOR_PID=101 M2_RUN_GUARD=12 M2_DEADLINE=$((SECONDS+60))
READY_TASK_B='{"lifecycle":"in_progress"}'
READY_DETAIL='{"diagnostics":{"runtime_report":{"failure":null},"incidents":[{"kind":"interrupted","detail":"SYNTHETIC_PRIVATE_DETAIL"}]},"prompt":"SYNTHETIC_PRIVATE_PROMPT"}'
DETAIL_UNAVAILABLE=false
ANSWER_CALLS=0

run_fixture() {
    jq -cn --arg a "$M2_TASK_A" --arg b "$M2_TASK_B" --arg writer "$M2_WRITER" \
        --arg ra "$RUN_A" --arg rb "$RUN_B" --arg observed "$1" --arg desired "$2" '
        {items:[{id:$ra,task_id:$a,employee_id:$writer,stage_id:"work",lease_fencing_token:1,
            environment_epoch:1,observed_state:"running",desired_state:"running"},
            {id:$rb,task_id:$b,employee_id:$writer,stage_id:"work",lease_fencing_token:2,
            environment_epoch:1,observed_state:$observed,desired_state:$desired}]}'
}
m1_process_token() { [[ $1 == 100 || $1 == 101 ]]; }
m2_command() { test_fail 'READY health check issued a management write'; }
m2_phase() { :; }
m2_record() { :; }
m2_answered() { ANSWER_CALLS=$((ANSWER_CALLS+1)); }
m2_prove_isolation() { :; }
sleep() { test_fail 'READY barrier slept despite a known failure or two ready healthy writers'; }
m2_pages() {
    [[ $1 == "/v1/projects/$M2_PROJECT_ID/runs" ]] || test_fail 'unexpected list route'
    printf '%s\n' "$READY_RUNS"
}
m2_read() {
    case "$1" in
        "/v1/projects/$M2_PROJECT_ID") printf '%s\n' '{"execution_gate":"open"}' ;;
        "/v1/projects/$M2_PROJECT_ID/tasks/$M2_TASK_A") printf '%s\n' '{"lifecycle":"in_progress"}' ;;
        "/v1/projects/$M2_PROJECT_ID/tasks/$M2_TASK_B") printf '%s\n' "$READY_TASK_B" ;;
        "/v1/projects/$M2_PROJECT_ID/runs/$RUN_A"|"/v1/projects/$M2_PROJECT_ID/runs/$RUN_B")
            [[ $DETAIL_UNAVAILABLE == false ]] || return 1
            jq -c --arg id "${1##*/}" --argjson details "$READY_DETAIL" '.items[]|select(.id==$id)|. + $details' <<<"$READY_RUNS" ;;
        *) test_fail 'unexpected detail route' ;;
    esac
}
assert_failed() {
    local name=$1 log="$TEST_ROOT/$1.log" answers=$ANSWER_CALLS
    if m2_ready_barrier >"$log" 2>&1; then test_fail "$name was accepted"; fi
    [[ $ANSWER_CALLS == "$answers" ]] || test_fail "$name continued into READY answers"
    rg -q 'READY прерван:' "$log" || test_fail "$name did not report the failed initial writer"
    rg -q "Task=$M2_TASK_B" "$log" || test_fail "$name missed Task identity"
    rg -q 'supervisor.log' "$log" || test_fail "$name lacks retained diagnostic pointer"
    ! rg -q 'SYNTHETIC_PRIVATE|180 секунд' "$log" || test_fail "$name leaked details or timed out"
    [[ $(wc -c <"$log") -lt 1500 ]] || test_fail "$name diagnostic was not bounded"
}

for state in failed stopped lost stopping unknown; do
    READY_RUNS=$(run_fixture "$state" running)
    assert_failed "observed-$state"
    rg -q "Run=$RUN_B.*observed=$state" "$TEST_ROOT/observed-$state.log" || test_fail 'actual Run state/ID missing'
done
for desired in failed stopped stop_requested force_stop_requested; do
    READY_RUNS=$(run_fixture running "$desired")
    assert_failed "desired-$desired"
done
READY_RUNS=$(run_fixture running running)
for state in waiting cancelled done; do
    READY_TASK_B=$(jq -cn --arg state "$state" '{lifecycle:$state,wait_conditions:[{kind:"interrupted",detail:"SYNTHETIC_PRIVATE_WAIT"}]}')
    assert_failed "task-$state"
done
rg -q 'reason_code=unavailable.*incident_kind=interrupted' "$TEST_ROOT/task-waiting.log" \
    || test_fail 'incident classification was misrepresented as the actual failure cause'
READY_TASK_B='{"lifecycle":"in_progress"}'
READY_RUNS=$(run_fixture unknown provision_requested)
m2_initial_writer_health "$READY_RUNS" || test_fail 'unobserved initial provision was treated as failure'
READY_RUNS=$(run_fixture provisioning provision_requested)
m2_initial_writer_health "$READY_RUNS" || test_fail 'normal provisioning was treated as failure'
READY_RUNS=$(run_fixture running running)
m2_ready_barrier >"$TEST_ROOT/healthy.log" 2>&1 || test_fail 'two healthy ready writers were rejected'
[[ $ANSWER_CALLS == 2 ]] || test_fail 'healthy barrier skipped exact READY checks'

READY_RUNS=$(run_fixture stopped stopped)
DETAIL_UNAVAILABLE=true
assert_failed detail-unavailable
DETAIL_UNAVAILABLE=false
# RuntimeReport.failure is a nullable string enum, not an object with a reason.
READY_DETAIL='{"diagnostics":{"runtime_report":{"failure":"provider_unavailable","message":"SYNTHETIC_PRIVATE_PROVIDER"},"incidents":[]}}'
assert_failed public-failure
rg -q 'reason_code=unavailable.*failure_kind=provider_unavailable' "$TEST_ROOT/public-failure.log" \
    || test_fail 'real public failure enum was not shown, or an unavailable reason was invented'
READY_DETAIL='{"diagnostics":{"runtime_report":{"failure":"bad\nSYNTHETIC_PRIVATE_KIND"},"incidents":[]}}'
assert_failed unsafe-code

# Old failures later in the workflow must not change the shared guard contract.
m2_guard || test_fail 'historical stopped Run broke the general/rework guard'
printf 'm2-ready-test: passed (keyless fail-fast/public-state regressions); retained %s\n' "$TEST_ROOT"
