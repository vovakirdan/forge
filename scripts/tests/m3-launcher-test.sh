#!/usr/bin/env bash
# Synthetic shell-contract tests. No credentials, services or provider calls.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$TEST_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
source "$SCRIPT_DIR/lib/m1-input.sh"
source "$SCRIPT_DIR/lib/m1-commands.sh"
source "$SCRIPT_DIR/lib/m3-session.sh"
source "$SCRIPT_DIR/lib/m3-seed.sh"
source "$SCRIPT_DIR/lib/m3-workflow.sh"
readonly TEST_ROOT="$(mktemp -d /tmp/forge-m3-launcher-test.XXXXXXXX)"
test_fail() { printf 'm3-launcher-test: %s; retained %s\n' "$1" "$TEST_ROOT" >&2; exit 1; }
M3_PLAYGROUND="$TEST_ROOT/playground"
install -d -m 700 "$M3_PLAYGROUND"
m3_prepare_playground
[[ -f $M3_PLAYGROUND/.git/info/exclude && -d $M3_PLAYGROUND/.forge-m3 ]] || test_fail 'empty Git template scaffold missing'
[[ -z $(env -i PATH=/usr/bin:/bin GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -C "$M3_PLAYGROUND" status --porcelain) ]] || test_fail 'private scaffold not ignored'
if m3_prepare_playground >/dev/null 2>&1; then test_fail 'used playground was accepted'; fi
m1_reset_launcher_state
M1_ROOT=$TEST_ROOT M1_PROJECT_ID=01990000-0000-7000-8000-000000000001
M1_AUTH="$M1_ROOT/auth.json" M1_MODEL=synthetic-model
M1_IMAGE="localhost/synthetic@sha256:$(printf '%064d' 0)"
M1_AUTH_SOURCE=$M1_AUTH M3_MODE=run M3_DEADLINE=$((SECONDS+60))
printf '{"tokens":{"account_id":"synthetic-account","access_token":"SYNTHETIC_SECRET_MUST_NOT_PRINT"}}\n' >"$M1_AUTH"
install -d -m 700 "$M1_ROOT/evidence"
: >"$TEST_ROOT/commands.jsonl"
m1_open_auth() { printf 'opened\n' >"$TEST_ROOT/opened"; }
if m3_confirm </dev/null >"$TEST_ROOT/unapproved.log" 2>&1; then test_fail 'unapproved noninteractive auth accepted'; fi
[[ ! -e $TEST_ROOT/opened ]] || test_fail 'auth read before approval'
M1_APPROVED=true
m3_confirm </dev/null >"$TEST_ROOT/approved.log"
[[ -f $TEST_ROOT/opened ]] || test_fail 'approved auth was not read'
M1_AUTH_SOURCE=''
if m3_confirm </dev/null >/dev/null 2>&1; then test_fail 'noninteractive missing auth accepted'; fi

readonly EMPLOYEE=01990000-0000-7000-8000-000000000002
readonly PIPELINE=01990000-0000-7000-8000-000000000003
readonly VERSION=01990000-0000-7000-8000-000000000004
m1_remove_import_copy() { printf 'removed\n' >"$TEST_ROOT/import-removed"; }
m1_command() {
    local kind=project id=$M1_PROJECT_ID
    jq -e 'has("operation")|not' <<<"$2" >/dev/null || test_fail 'caller supplied named-command operation'
    jq -cn --arg name "$1" --argjson payload "$2" '{name:$name,payload:$payload}' >>"$TEST_ROOT/commands.jsonl"
    case "$1" in create_employee) kind=employee; id=$EMPLOYEE ;; create_pipeline) kind=pipeline; id=$PIPELINE ;; esac
    jq -cn --arg kind "$kind" --arg id "$id" '{status:"applied",resource:{kind:$kind,id:$id}}'
}
m3_profile_template() {
    printf '%s\n' "$1" >"$TEST_ROOT/template.json"
    jq '{employee_id,binding:{surface:{mode:"filesystem_sandbox"},access,limits,execution_profile:{id:"synthetic"},system_prompt,employee_prompt}}' <<<"$1"
}
m1_read() {
    case "$1" in
        */onboarding) printf '{"state":"pending"}\n' ;;
        */pipelines) jq -cn --arg id "$PIPELINE" --arg version "$VERSION" '{items:[{pipeline_id:$id,id:$version}]}' ;;
        */runs*) jq -cn --argjson count "${SYNTHETIC_RUN_COUNT:-0}" '{items:[range($count)|{id:.}],next_cursor:null}' ;;
        *) printf '{}\n' ;;
    esac
}
m3_seed_employee >"$TEST_ROOT/seed.log"
M1_TASK_ID=01990000-0000-7000-8000-000000000005
m3_request_summary
jq -se '([.[]|.name]|index("configure_system_jobs"))<([.[]|.name]|index("create_employee")) and
    all(.[]; (.payload|has("operation")|not)) and
    all(.[];.name!="skip_employee_onboarding" and .name!="start_project_execution" and .name!="submit_outcome") and
    any(.[];.name=="configure_system_jobs" and .payload.binding.surface.mode=="none" and .payload.policy.max_attempts_per_job==1 and .payload.policy.max_attempts_per_day==4) and
    any(.[];.name=="request_task_summary" and (.payload|keys)==["task_id"]) and
    any(.[];.name=="configure_employee_runtime" and .payload.binding.surface.mode=="filesystem_sandbox")' "$TEST_ROOT/commands.jsonl" >/dev/null || test_fail 'wrong onboarding/ownership/allowance sequence'
[[ $M1_EMPLOYEE_ID == "$EMPLOYEE" && -f $TEST_ROOT/import-removed ]] || test_fail 'enrollment did not preserve intended identity/cleanup'
! rg -q SYNTHETIC_SECRET_MUST_NOT_PRINT "$TEST_ROOT/commands.jsonl" "$TEST_ROOT/template.json" "$TEST_ROOT/seed.log" "$TEST_ROOT/approved.log" || test_fail 'source token leaked'
SYNTHETIC_RUN_COUNT=5 m3_guard || test_fail 'guard stopped below configured threshold'
if SYNTHETIC_RUN_COUNT=6 m3_guard >/dev/null 2>&1; then test_fail 'observed Run cap ignored'; fi
M3_CONTAINER="$(printf '%064d' 1)" M1_HOST=synthetic-host
podman() { printf 'foreign-host\n'; }
timeout() { test_fail 'foreign container reached mutation'; }
if m3_index_control stop >/dev/null 2>&1; then test_fail 'foreign index container accepted'; fi

# Reproduce EXIT's conditional invocation: do not rely on errexit in m3_collect.
test_collection() (
    local fault=$1
    M3_MODE=index M3_PHASE=passed FORGE_POSTGRES_USER=synthetic M1_DATABASE=synthetic
    m1_read() {
        case "$1" in
            */runs\?*)
                case "$fault" in
                    runs_read) return 1 ;;
                    runs_json) printf 'broken json\n'; return 0 ;;
                    runs_cursor) printf '{"items":[],"next_cursor":"more"}\n'; return 0 ;;
                esac
                jq -cn --arg id "$EMPLOYEE" '{items:[{id:$id}],next_cursor:null}' ;;
            */system-jobs)
                [[ $fault != jobs_read ]] || return 1
                printf '{"jobs":[]}\n' ;;
            */memory/status)
                [[ $fault != status_read ]] || return 1
                printf '{"configured":true}\n' ;;
            */runs/*)
                [[ $fault != run_read ]] || return 1
                if [[ $fault == run_json ]]; then printf 'broken json\n'; else printf '{}\n'; fi ;;
            *) return 1 ;;
        esac
    }
    service_container_id() {
        [[ $fault != container ]] || return 1
        printf 'synthetic-postgres\n'
    }
    timeout() {
        [[ $fault != database ]] || return 1
        if [[ $fault == database_json ]]; then printf 'broken json\n'; else printf '{}\n'; fi
    }
    if m3_collect >"$TEST_ROOT/collect-$fault.log" 2>&1; then
        [[ $fault == none ]] || test_fail "evidence failure swallowed: $fault"
    else
        [[ $fault != none ]] || test_fail 'valid evidence collection failed'
    fi
)
for fault in none runs_read runs_json runs_cursor jobs_read status_read run_read run_json container database database_json; do
    test_collection "$fault"
done
for suite in m3_knowledge m3_memory_projection m3_system_job_ownership m3_system_job_lifecycle \
    m3_context m3_system_job_gateway m3_system_job_sources m3_onboarding_admission m3_system_job_restart; do
    rg -q -- "--test $suite([[:space:]]|$)" "$SCRIPT_DIR/run-m3.sh" || test_fail "keyless suite omitted: $suite"
done
printf 'M3 synthetic launcher contracts passed; retained %s\n' "$TEST_ROOT"
