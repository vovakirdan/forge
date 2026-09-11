#!/usr/bin/env bash
set -euo pipefail
umask 077

TEST_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
FORGE_ROOT_DIR=$(CDPATH= cd -- "$TEST_SCRIPT_DIR/../.." && pwd -P)
# shellcheck source=../lib/m1-commands.sh
source "$FORGE_ROOT_DIR/scripts/lib/m1-commands.sh"
TEST_ROOT=$(mktemp -d /tmp/forge-m1-command-test.XXXXXX)
trap 'rm -rf -- "$TEST_ROOT"' EXIT
M1_ROOT=$TEST_ROOT
M1_AUTH=$TEST_ROOT/synthetic-auth.json
M1_MODEL=synthetic-model
M1_IMAGE="localhost/synthetic@sha256:$(printf '%064d' 0)"
M1_TASK_TEXT='Create and verify a small example.'
TEST_PIPELINE=01990000-0000-7000-8000-000000000001
TEST_VERSION=01990000-0000-7000-8000-000000000002
TEST_EMPLOYEE=01990000-0000-7000-8000-000000000003
TEST_TASK=01990000-0000-7000-8000-000000000004
TEST_ARTIFACT=01990000-0000-7000-8000-000000000005
TEST_RUN=01990000-0000-7000-8000-000000000006
TEST_MODE=seed
printf '%s\n' '{"tokens":{"account_id":"synthetic-account","access_token":"SYNTHETIC_TOKEN_DO_NOT_PRINT"}}' >"$M1_AUTH"

test_fail() { printf 'm1-commands-test: %s\n' "$1" >&2; exit 1; }
test_reset() { : >"$TEST_ROOT/calls"; : >"$TEST_ROOT/count"; }

# This replaces the transport completely: no Core, auth enrollment or provider.
m1_cli() {
    local action=$1 path=${2:-} name key= revision= payload= count=0 id kind
    if [[ $action == get ]]; then
        case "$path" in
            */tasks/*) jq -cn --arg id "$TEST_TASK" --arg artifact "$TEST_ARTIFACT" \
                '{id:$id,revision:1,lifecycle:"waiting",description:"SYNTHETIC_TOKEN_DO_NOT_PRINT",
                  artifacts:[{id:$artifact,kind:"stage_evidence",title:"SYNTHETIC_TOKEN_DO_NOT_PRINT",
                    metadata:{token:"SYNTHETIC_TOKEN_DO_NOT_PRINT"},body:"SYNTHETIC_TOKEN_DO_NOT_PRINT"}],wait_conditions:[]}' ;;
            */pipelines) jq -cn --arg id "$TEST_VERSION" --arg pipeline "$TEST_PIPELINE" \
                '{items:[{id:$id,pipeline_id:$pipeline}]}' ;;
            */runs/*) jq -cn --arg id "$TEST_RUN" \
                '{id:$id,desired_state:"stopped",observed_state:"stopped",diagnostics:{
                  runtime_report:{secret:"SYNTHETIC_TOKEN_DO_NOT_PRINT"},evidence:[{}],incidents:[],
                  handoff:{secret:"SYNTHETIC_TOKEN_DO_NOT_PRINT"},streams:[{incomplete:true}]}}' ;;
            */runs) jq -cn --arg id "$TEST_RUN" --arg task "$TEST_TASK" '{items:[{id:$id,task_id:$task}]}' ;;
            *)
                [[ ! -s $TEST_ROOT/count ]] || read -r count <"$TEST_ROOT/count"
                jq -cn --argjson revision "$((count + 1))" '{revision:$revision,execution_gate:"stopped"}' ;;
        esac
        return
    fi
    [[ $action == command ]] || test_fail 'unexpected transport call'
    name=$2; shift 2
    while (($#)); do
        case "$1" in
            --idempotency-key) key=$2 ;;
            --expected-revision) revision=$2 ;;
            --payload) payload=$2 ;;
        esac
        shift 2
    done
    [[ ! -s $TEST_ROOT/count ]] || read -r count <"$TEST_ROOT/count"
    count=$((count + 1)); printf '%s\n' "$count" >"$TEST_ROOT/count"
    jq -cn --arg name "$name" --arg key "$key" --argjson revision "$revision" \
        --argjson payload "$payload" '{name:$name,key:$key,revision:$revision,payload:$payload}' >>"$TEST_ROOT/calls"
    if [[ $TEST_MODE == unknown ]]; then
        printf 'SYNTHETIC_TOKEN_DO_NOT_PRINT timeout after possible commit\n' >&2
        return 124
    fi
    if [[ $TEST_MODE == conflict && $count == 1 ]]; then
        printf 'forge: local Core API returned HTTP 409: domain transition rejected: stale project revision: expected %s, current 2\n' "$revision" >&2
        return 1
    fi
    if [[ $TEST_MODE == false_conflict ]]; then
        printf 'forge: local Core API returned HTTP 409: idempotency key was reused for a different command\n' >&2
        return 1
    fi
    case "$name" in
        create_pipeline) id=$TEST_PIPELINE; kind=pipeline ;;
        create_employee) id=$TEST_EMPLOYEE; kind=employee ;;
        create_task) id=$TEST_TASK; kind=task ;;
        *) id=${M1_PROJECT_ID:-$TEST_TASK}; kind=project ;;
    esac
    jq -cn --arg id "$id" --arg kind "$kind" \
        '{status:"applied",project_revision:2,resource:{id:$id,kind:$kind}}'
}

for ((index = 0; index < 20; index++)); do
    value=$(m1_uuid7)
    [[ $value =~ ^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$ ]] || test_fail 'UUIDv7 format'
    printf '%s\n' "$value"
done >"$TEST_ROOT/ids"
[[ $(sort -u "$TEST_ROOT/ids" | wc -l) == 20 ]] || test_fail 'UUIDv7 uniqueness'

test_reset
m1_seed >"$TEST_ROOT/output" 2>"$TEST_ROOT/error" || test_fail 'seed failed'
[[ $M1_PROJECT_ID && $M1_TASK_ID == "$TEST_TASK" && $M1_EMPLOYEE_ID == "$TEST_EMPLOYEE" ]] || test_fail 'seed globals lost'
jq -se 'map(.name) == ["create_project","create_pipeline","create_employee","skip_employee_onboarding","enroll_credential",
    "configure_employee_runtime","create_task","approve_task"]' "$TEST_ROOT/calls" >/dev/null || test_fail 'seed must never start project'
jq -se --arg employee "$TEST_EMPLOYEE" '.[3].payload | .employee_id == $employee and
    (.reason | type == "string" and length > 0)' "$TEST_ROOT/calls" >/dev/null || test_fail 'historical seed must explicitly skip onboarding'
jq -se '.[1].payload.transitions[0].artifact_requirements[0] ==
    {kind:"stage_evidence",minimum_count:1,scope:"current_stage"}' "$TEST_ROOT/calls" >/dev/null || test_fail 'missing evidence requirement'
jq -se '.[5].payload.binding | .surface == {mode:"filesystem_sandbox"} and .access == "read_write" and
    .limits == {cpu_millis:2000,memory_bytes:4294967296,pids:256,wall_seconds:600,stop_grace_seconds:5} and
    .execution_profile.adapter_version == "0.153.2" and .execution_profile.model == "synthetic-model" and
    .execution_profile.credential_binding.account_id == "synthetic-account" and
    (.execution_profile.capability_profile.capabilities | index("native_mcp")) != null and
    (.employee_prompt | contains("forge_submit_artifact") and contains("forge_submit_outcome") and
        contains("artifact_submission_message_ids") and contains("forge_request_human"))' \
    "$TEST_ROOT/calls" >/dev/null || test_fail 'runtime profile mismatch'
! rg -q 'SYNTHETIC_TOKEN_DO_NOT_PRINT|synthetic-account' "$TEST_ROOT/session.json" "$TEST_ROOT/output" "$TEST_ROOT/error" || test_fail 'seed exposed credential fields'
[[ $(stat -c %a "$TEST_ROOT/session.json") == 600 ]] || test_fail 'session file is not private'

TEST_MODE=conflict; test_reset
m1_start_project >/dev/null || test_fail 'known stale revision should retry'
jq -se 'length == 2 and .[0].key == .[1].key and .[0].revision == 1 and .[1].revision == 2' \
    "$TEST_ROOT/calls" >/dev/null || test_fail 'retry changed identity or missed fresh revision'
for TEST_MODE in unknown false_conflict; do
    test_reset
    if m1_start_project >"$TEST_ROOT/output" 2>"$TEST_ROOT/error"; then test_fail 'failure treated as accepted'; fi
    [[ $(wc -l <"$TEST_ROOT/calls") == 1 ]] || test_fail 'unconfirmed command retried'
    ! rg -q 'SYNTHETIC_TOKEN_DO_NOT_PRINT' "$TEST_ROOT/output" "$TEST_ROOT/error" || test_fail 'error exposed transport text'
done

TEST_MODE=seed; test_reset
m1_stop_project || test_fail 'named stop failed'
jq -se 'length == 1 and .[0].name == "stop_project_execution"' "$TEST_ROOT/calls" >/dev/null || test_fail 'missing named stop'
m1_show_result >"$TEST_ROOT/output" || test_fail 'result read failed'
! rg -q 'SYNTHETIC_TOKEN_DO_NOT_PRINT|synthetic-account' "$TEST_ROOT/output" || test_fail 'summary exposed untrusted prose'
jq -se '.[0].lifecycle == "waiting" and .[0].artifact_count == 1 and .[1].incomplete_stream_count == 1' \
    "$TEST_ROOT/output" >/dev/null || test_fail 'summary changed outcome or omitted diagnostic counts'
printf 'm1-commands-test: passed (synthetic transport only)\n'
