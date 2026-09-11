#!/usr/bin/env bash
# Bulk public JSON uses stdin, never process arguments. No services or providers.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$TEST_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/lib/m2-common.sh"
source "$SCRIPT_DIR/lib/m2-workflow.sh"
source "$SCRIPT_DIR/lib/m2-evidence.sh"
TEST_ROOT=$(mktemp -d /tmp/forge-m2-json-test.XXXXXXXX)
test_fail() { printf 'm2-json-test: %s\nRetained: %s\n' "$1" "$TEST_ROOT" >&2; exit 1; }
M2_ROOT=$TEST_ROOT M2_PROJECT_ID=project M2_WRITER=writer
install -d -m 700 "$TEST_ROOT/fixtures" "$TEST_ROOT/evidence"
MODE=single
m1_cli() {
    [[ $1 == watch && $4 == --after ]] || test_fail 'unexpected CLI call'
    local cursor=$5
    case "$MODE" in
        single) ((cursor != 0)) || command jq -c . "$TEST_ROOT/fixtures/large-event.json" ;;
        multi) ((cursor >= 19)) || command jq -c --argjson i "$((cursor+1))" '.project_sequence=$i' "$TEST_ROOT/fixtures/small-event.json" ;;
        fail) printf '%s\n' '{"project_sequence":99}'; return 1 ;;
        malformed) printf '%s\n' '{SYNTHETIC_BODY_BROKEN' ;;
        repeated) printf '%s\n' '{"project_sequence":1}' ;;
        unordered) printf '%s\n' '{"project_sequence":2}' '{"project_sequence":1}' ;;
        invalid) printf '%s\n' '{"project_sequence":"SYNTHETIC_BODY"}' ;;
    esac
}
command jq -cn '{project_sequence:1,payload:{body:(("x"*160000)+"SYNTHETIC_BODY α\"\n")}}' >"$TEST_ROOT/fixtures/large-event.json"
command jq -cn '{project_sequence:1,payload:{body:(("x"*120000)+"SYNTHETIC_BODY α\"\n")}}' >"$TEST_ROOT/fixtures/small-event.json"
m2_events >"$TEST_ROOT/events-single.json" 2>"$TEST_ROOT/single.err" || test_fail 'single large event page failed'
command jq -e --slurpfile expected "$TEST_ROOT/fixtures/large-event.json" '.==$expected' "$TEST_ROOT/events-single.json" >/dev/null || test_fail 'single-page body changed'

# Catch a smaller body sent through argv too, without ever printing its value.
jq() {
    local argument
    for argument in "$@"; do [[ $argument != *SYNTHETIC_BODY* ]] || test_fail 'body leaked into jq argv'; done
    command jq "$@"
}
MODE=multi
m2_events >"$TEST_ROOT/events-multi.json" || test_fail 'accumulated events failed'
[[ $(wc -c <"$TEST_ROOT/events-multi.json") -gt 2097152 ]] || test_fail 'event fixture did not cross ARG_MAX'
command jq -e --slurpfile expected "$TEST_ROOT/fixtures/small-event.json" '
    length==19 and [.[].project_sequence]==[range(1;20)] and all(.[];.payload==$expected[0].payload)' \
    "$TEST_ROOT/events-multi.json" >/dev/null || test_fail 'events lost order or body'
for MODE in fail malformed repeated unordered invalid; do
    if m2_events >"$TEST_ROOT/$MODE.json" 2>"$TEST_ROOT/$MODE.err"; then test_fail "$MODE events accepted"; fi
    [[ ! -s $TEST_ROOT/$MODE.json ]] || test_fail "$MODE exposed partial events"
    ! rg -q SYNTHETIC_BODY "$TEST_ROOT/$MODE.err" || test_fail "$MODE diagnostic exposed body"
done

m2_read() {
    local cursor=0 next=null
    if [[ $1 == "/v1/projects/$M2_PROJECT_ID" ]]; then
        if [[ $MODE == collect-project-fail ]]; then printf '%s\n' '{"partial":true}'; return 1; fi
        command jq -c '.payload + {execution_gate:"open"}' "$TEST_ROOT/fixtures/large-event.json"
        return
    fi
    if [[ $MODE == collect ]]; then printf '%s\n' '{"items":[],"next_cursor":null}'; return; fi
    if [[ $1 == *'&after='* ]]; then cursor=${1##*&after=}; cursor=$((16#$cursor)); fi
    case "$MODE" in
        page-single) command jq -c '{items:[.payload],next_cursor:null}' "$TEST_ROOT/fixtures/large-event.json" ;;
        page-multi)
            if ((cursor < 18)); then printf -v next '"%08x"' "$((cursor+1))"; fi
            command jq -c --argjson next "$next" --argjson i "$cursor" '{items:[(.payload+{index:$i})],next_cursor:$next}' "$TEST_ROOT/fixtures/small-event.json" ;;
        page-repeat) printf '%s\n' '{"items":[],"next_cursor":"00000001"}' ;;
        page-invalid) printf '%s\n' '{"items":{},"next_cursor":null}' ;;
        page-invalid-item) printf '%s\n' '{"items":["SYNTHETIC_BODY"],"next_cursor":null}' ;;
        page-invalid-cursor) printf '%s\n' '{"items":[],"next_cursor":17}' ;;
        page-multiple) printf '%s\n' '{"items":[],"next_cursor":null}' '{"items":[{"body":"SYNTHETIC_BODY"}],"next_cursor":null}' ;;
        page-fail) printf '%s\n' '{"items":[]}'; return 1 ;;
    esac
}
MODE=page-single
m2_pages /items >"$TEST_ROOT/pages-single.json" || test_fail 'single large list page failed'
MODE=page-multi
m2_pages /items >"$TEST_ROOT/pages-multi.json" || test_fail 'large list aggregation failed'
[[ $(wc -c <"$TEST_ROOT/pages-multi.json") -gt 2097152 ]] || test_fail 'list fixture did not cross ARG_MAX'
command jq -e --slurpfile expected "$TEST_ROOT/fixtures/small-event.json" '
    (.items|length)==19 and [.items[].index]==[range(19)] and all(.items[];.body==$expected[0].payload.body)' \
    "$TEST_ROOT/pages-multi.json" >/dev/null || test_fail 'lists lost order or body'
for MODE in page-repeat page-invalid page-invalid-item page-invalid-cursor page-multiple page-fail; do
    if m2_pages /items >"$TEST_ROOT/$MODE.json" 2>"$TEST_ROOT/$MODE.err"; then test_fail "$MODE accepted"; fi
    [[ ! -s $TEST_ROOT/$MODE.json ]] || test_fail "$MODE exposed partial list"
    ! rg -q SYNTHETIC_BODY "$TEST_ROOT/$MODE.err" || test_fail "$MODE diagnostic exposed body"
done

for invalid in '' '{SYNTHETIC_BODY' '{} {}' '"SYNTHETIC_BODY"' null; do
    m2_json_write "$TEST_ROOT/previous.json" '{"retained":true}'
    if m2_json_write "$TEST_ROOT/previous.json" "$invalid" 2>"$TEST_ROOT/write.err"; then test_fail 'invalid snapshot accepted'; fi
    command jq -e '.retained==true' "$TEST_ROOT/previous.json" >/dev/null || test_fail 'old snapshot overwritten'
    ! rg -q SYNTHETIC_BODY "$TEST_ROOT/write.err" || test_fail 'write diagnostic exposed body'
done
m2_json_write "$TEST_ROOT/session.json" '{"task_thread_ids":[]}'
m2_json_write "$TEST_ROOT/evidence/events.json" '[{"retained":true}]'
MODE=collect
# A failed source may emit valid-looking partial JSON; its status must survive.
m2_events() { printf '%s\n' '[]'; return 1; }
if m2_collect >"$TEST_ROOT/collect.log" 2>&1; then test_fail 'failed collection reported success'; fi
command jq -e '.[0].retained==true' "$TEST_ROOT/evidence/events.json" >/dev/null || test_fail 'failed collection replaced evidence'
MODE=collect-project-fail
m2_json_write "$TEST_ROOT/evidence/project.json" '{"retained":true}'
if m2_collect >"$TEST_ROOT/project-collect.log" 2>&1; then test_fail 'failed Project read reported success'; fi
command jq -e '.retained==true' "$TEST_ROOT/evidence/project.json" >/dev/null || test_fail 'failed Project read replaced evidence'

TASKS=$(command jq -c '{items:[(.payload+{id:"task",lifecycle:"in_progress",current_stage_id:"work"})]}' "$TEST_ROOT/fixtures/large-event.json")
RUNS=$(command jq -c '{items:[(.payload+{task_id:"task",stage_id:"work",observed_state:"running"})]}' "$TEST_ROOT/fixtures/large-event.json")
m2_progress "$TASKS" "$RUNS" 60 >"$TEST_ROOT/progress.log" || test_fail 'large progress projections failed'
rg -q 'running=1' "$TEST_ROOT/progress.log" || test_fail 'progress summary changed'
! rg -q SYNTHETIC_BODY "$TEST_ROOT/progress.log" || test_fail 'progress exposed body'
PROJECT=$(command jq -c '.payload+{id:"project",execution_gate:"open"}' "$TEST_ROOT/fixtures/large-event.json")
m2_status_projection "$PROJECT" "$TASKS" >"$TEST_ROOT/status.json" || test_fail 'large status projections failed'
command jq -e --slurpfile expected "$TEST_ROOT/fixtures/large-event.json" '
    .project.body==$expected[0].payload.body and .tasks==[{id:"task",key:null,lifecycle:"in_progress",current_stage_id:"work"}]' \
    "$TEST_ROOT/status.json" >/dev/null || test_fail 'status projection changed'
M1_LAUNCHER_PID=$BASHPID M2_TARGET="$TEST_ROOT/target.git"
m1_process_token() { printf '%s\n' synthetic-token; }
PATCH=$(command jq -c '{note:.payload.body}' "$TEST_ROOT/fixtures/large-event.json")
m2_record "$PATCH" || test_fail 'large session patch failed'
m2_record '{"phase":"checked"}' || test_fail 'large retained session failed'
command jq -e '.phase=="checked" and (.note|contains("SYNTHETIC_BODY"))' "$TEST_ROOT/session.json" >/dev/null || test_fail 'session patch data changed'
if m2_record '"SYNTHETIC_BODY"' 2>"$TEST_ROOT/record.err"; then test_fail 'invalid session patch accepted'; fi
! rg -q SYNTHETIC_BODY "$TEST_ROOT/record.err" || test_fail 'session patch diagnostic exposed body'

M2_TASK_A=a M2_TASK_B=b M1_EXEC="$TEST_ROOT/exec"
LARGE_RUNS=$(command jq -c '{items:[range(2) as $i | .payload+{id:(if $i==0 then "a" else "b" end),
    task_id:(if $i==0 then "a" else "b" end),employee_id:"writer",stage_id:"work",observed_state:"running",
    lease_fencing_token:1,environment_epoch:1}]}' "$TEST_ROOT/fixtures/large-event.json")
m2_first_runs "$LARGE_RUNS" || test_fail 'large first Run projections failed'
m1_container_state() { printf '%s\n' running; }
m1_podman() {
    case "${*: -1}" in
        forge-run-a-*) printf '[{"Destination":"/workspace","Source":"%s/surfaces/a"}]\n' "$M1_EXEC" ;;
        forge-run-b-*) printf '[{"Destination":"/workspace","Source":"%s/surfaces/b"}]\n' "$M1_EXEC" ;;
        *) test_fail 'unexpected container inspection' ;;
    esac
}
m2_prove_isolation || test_fail 'large overlap projections failed'
command jq -e '.owned_containers_running and (.runs|length)==2' "$TEST_ROOT/evidence/initial-overlap.json" >/dev/null \
    || test_fail 'overlap evidence changed'
printf 'm2-json-test: passed (bulk stdin, pagination validation and retained evidence); retained %s\n' "$TEST_ROOT"
