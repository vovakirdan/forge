#!/usr/bin/env bash
# Exercise the real EXIT trap/report functions with synthetic cleanup and evidence.
# No provider, service, process signalling or user-session paths are used.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$TEST_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/lib/m2-common.sh"
source "$SCRIPT_DIR/lib/m2-session.sh"
source "$SCRIPT_DIR/lib/m2-evidence.sh"

TEST_ROOT=$(mktemp -d /tmp/forge-m2-report-test.XXXXXXXX)
test_fail() { printf 'm2-report-test: %s\nRetained: %s\n' "$1" "$TEST_ROOT" >&2; exit 1; }
TARGET_FIXTURE="$TEST_ROOT/target.json"
jq -cn '{commit:("a"*40),tree:[]}' >"$TARGET_FIXTURE"

trace() { printf '%s\n' "$1" >>"$M2_ROOT/calls"; }
m1_close_auth() { trace close_auth; }
m1_stop_project() { trace stop_project; return "$STOP_CODE"; }
m2_await_stopped() { trace await_stopped; return "$AWAIT_CODE"; }
m1_cleanup() { trace cleanup; return "$CLEANUP_CODE"; }
m1_remove_import_copy() { test_fail 'unexpected auth import removal'; }
m2_collect() {
    trace collect
    m2_json_write "$M2_ROOT/evidence/partial.json" '{"partial_snapshot":true}' || return 1
    return "$COLLECTION_CODE"
}
m2_target_evidence() {
    trace target
    ((TARGET_CODE == 0)) || return 1
    jq -c . "$TARGET_FIXTURE"
}
m2_assess() {
    trace assess
    [[ $1 == 0 ]] || test_fail 'assessment received failed cleanup'
    return "$ASSESSMENT_CODE"
}
m2_record() { trace record; m2_json_write "$M2_ROOT/session.json" "$1"; }

run_exit_case() {
    local name=$1 workflow_code=$2 COLLECTION_CODE=$3 CLEANUP_CODE=$4 ASSESSMENT_CODE=$5
    local started=${6:-true} STOP_CODE=${7:-0} AWAIT_CODE=${8:-0} TARGET_CODE=${9:-0}
    local root="$TEST_ROOT/$name" actual expected=$workflow_code cleanup=passed evidence=passed
    local outcome=failed assessment=unverified workflow=failed assessed=0 calls
    install -d -m 700 "$root/evidence"
    # A partial refresh must not retain or re-use an earlier successful report.
    m2_json_write "$root/evidence/report.json" '{"outcome":"passed","assessment":"passed","cleanup":"passed"}'
    if (
        M2_ROOT=$root M2_STARTED=$started M2_LANE=synthetic M2_MODEL=no-inference
        M1_CORE_PID=synthetic-core M1_SUPERVISOR_PID=synthetic-supervisor M1_AUTH=''
        trap m2_on_exit EXIT
        exit "$workflow_code"
    ) >"$root/output.log" 2>&1; then actual=0; else actual=$?; fi

    if ((CLEANUP_CODE != 0 || STOP_CODE != 0 || AWAIT_CODE != 0)); then cleanup=failed; fi
    if [[ -z $started ]]; then evidence=unverified; outcome=unverified; workflow=unverified
    else
        ((workflow_code != 0)) || workflow=passed
        ((COLLECTION_CODE == 0)) || evidence=failed
    fi
    ((TARGET_CODE == 0)) || evidence=failed
    if [[ $cleanup == failed || $evidence == failed ]]; then ((expected != 0)) || expected=1; fi
    if [[ -n $started ]]; then
        if ((workflow_code == 0)) && [[ $cleanup == passed && $evidence == passed ]]; then
            assessed=1
            if ((ASSESSMENT_CODE == 0)); then assessment=passed; outcome=passed
            else assessment=failed; expected=1; fi
        else
            ((expected != 0)) || expected=1
        fi
    fi
    [[ $actual == "$expected" ]] || test_fail "$name exit=$actual expected=$expected"
    jq -e --arg outcome "$outcome" --arg assessment "$assessment" --arg cleanup "$cleanup" \
        --arg evidence "$evidence" --arg workflow "$workflow" \
        --argjson original "$workflow_code" --argjson exit "$expected" '
        .outcome==$outcome and .assessment==$assessment and .cleanup==$cleanup and
        .evidence_collection==$evidence and .workflow==$workflow and
        .workflow_exit_status==$original and .exit_status==$exit' \
        "$root/evidence/report.json" >/dev/null || test_fail "$name report disagrees with independent outcomes"
    jq -e --arg outcome "$outcome" '.phase=="closed" and .outcome==$outcome' \
        "$root/session.json" >/dev/null || test_fail "$name did not retain the closed session"
    calls=$(tr '\n' ' ' <"$root/calls")
    if [[ -n $started ]]; then
        [[ $calls == "close_auth stop_project await_stopped collect cleanup target "* ]] \
            || test_fail "$name skipped or reordered cleanup after collection"
    else
        [[ $calls == 'close_auth cleanup target record ' ]] || test_fail "$name collected an unstarted workflow"
    fi
    if ((assessed == 1)); then
        [[ $calls == *'target assess record ' ]] || test_fail "$name did not assess complete evidence"
    else
        [[ $calls != *'assess '* ]] || test_fail "$name assessed missing, partial or untrusted evidence"
    fi
    rg -Fq "evidence collection: $evidence; cleanup: $cleanup" "$root/output.log" \
        || test_fail "$name display conflated evidence and cleanup"
}

for workflow in 0 1; do
    for collection in 0 1; do
        for cleanup in 0 1; do
            for assessment in 0 1; do
                run_exit_case "matrix-$workflow-$collection-$cleanup-$assessment" \
                    "$workflow" "$collection" "$cleanup" "$assessment"
            done
        done
    done
done
run_exit_case interrupted 130 0 0 0
run_exit_case stop-failed 0 0 0 0 true 1
run_exit_case await-failed 0 0 0 0 true 0 1
run_exit_case target-failed 0 0 0 0 true 0 0 1
run_exit_case unstarted 0 0 0 0 ''
run_exit_case unstarted-workflow-failed 1 0 0 0 ''
run_exit_case unstarted-cleanup-failed 0 0 1 0 ''

# More than Linux's per-argument limit: target JSON must travel through stdin.
TARGET_FIXTURE="$TEST_ROOT/large-target.json"
jq -cn '{commit:("a"*40),tree:[range(0;4096)|{mode:"100644",kind:"blob",object:("b"*40),path:("file-"+tostring+("x"*64))}]}' \
    >"$TARGET_FIXTURE"
run_exit_case large-target 0 0 0 0
jq -e '.target.tree|length==4096' "$TEST_ROOT/large-target/evidence/report.json" >/dev/null \
    || test_fail 'large target evidence was lost or truncated'

# Calling the reporter without proof of successful collection is conservative.
M2_ROOT="$TEST_ROOT/missing-collection" M2_STARTED=true M2_LANE=synthetic M2_MODEL=no-inference
TARGET_CODE=0 ASSESSMENT_CODE=0
install -d -m 700 "$M2_ROOT/evidence"
if m2_finish_report 0 0 >"$M2_ROOT/output.log" 2>&1; then test_fail 'missing collection default passed'; fi
jq -e '.outcome=="failed" and .assessment=="unverified" and .evidence_collection=="unverified" and .cleanup=="passed"' \
    "$M2_ROOT/evidence/report.json" >/dev/null || test_fail 'missing collection status was guessed'
! rg -qx assess "$M2_ROOT/calls" || test_fail 'default missing collection invoked assessment'

m2_load_session() { :; }
M2_ROOT="$TEST_ROOT/matrix-0-1-0-0" M2_TARGET=synthetic-target
m2_evidence_command >"$TEST_ROOT/display.log"
rg -q '"evidence_collection": "failed"' "$TEST_ROOT/display.log" || test_fail 'evidence command hid collection failure'
rg -q '"cleanup": "passed"' "$TEST_ROOT/display.log" || test_fail 'evidence command misreported successful cleanup'
printf 'm2-report-test: passed (synthetic EXIT trap, independent outcomes and large target); retained %s\n' "$TEST_ROOT"
