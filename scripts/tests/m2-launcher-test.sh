#!/usr/bin/env bash
# No credentials, services, containers or model calls; transport is synthetic.
set -euo pipefail
umask 077
readonly TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$TEST_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
source "$SCRIPT_DIR/lib/m1-commands.sh"
source "$SCRIPT_DIR/lib/m1-session.sh"
source "$SCRIPT_DIR/lib/m2-common.sh"
source "$SCRIPT_DIR/lib/m2-config.sh"
source "$SCRIPT_DIR/lib/m2-session.sh"
source "$SCRIPT_DIR/lib/m2-seed.sh"
source "$SCRIPT_DIR/lib/m2-workflow.sh"
source "$SCRIPT_DIR/lib/m2-evidence.sh"

TEST_ROOT=$(mktemp -d /tmp/forge-m2-launcher-test.XXXXXXXX)
test_fail() { printf 'm2-launcher-test: %s\nRetained: %s\n' "$1" "$TEST_ROOT" >&2; exit 1; }
printf 'Retained synthetic launcher tests: %s\n' "$TEST_ROOT" >&2

M2_PROJECT_DIR="$TEST_ROOT/project"
install -d -m 700 "$M2_PROJECT_DIR"
m2_git init --quiet --template= --initial-branch=main "$M2_PROJECT_DIR"
M2_PRIVATE="$M2_PROJECT_DIR/.forge-m2" M2_TARGET="$M2_PRIVATE/target.git"
m2_init >"$TEST_ROOT/init.log"
[[ -x $M2_PROJECT_DIR/forge-m2 && -f $M2_PRIVATE/config.json ]] || test_fail 'missing scaffold'
[[ $(stat -c %a "$M2_PRIVATE/config.json") == 600 && $(stat -c %a "$M2_PRIVATE") == 700 ]] || test_fail 'private modes'
[[ -z $(m2_git -C "$M2_PROJECT_DIR" status --porcelain) ]] || test_fail 'scaffold not ignored'
! m2_git -C "$M2_PROJECT_DIR" rev-parse --verify HEAD >/dev/null 2>&1 || test_fail 'fake checkout commit'
[[ -z $(m2_git --git-dir="$M2_TARGET" for-each-ref --format='%(refname)') ]] || test_fail 'target is not unborn'
[[ $(m2_git --git-dir="$M2_TARGET" rev-list --all --count) == 0 ]] || test_fail 'fake target commit'
"$M2_PROJECT_DIR/forge-m2" --help >"$TEST_ROOT/help.log"
if m2_init >"$TEST_ROOT/reinit.log" 2>&1; then test_fail 'init overwrote existing scaffold'; fi

M2_ROOT="$TEST_ROOT/session"
install -d -m 700 "$M2_ROOT" "$M2_ROOT/evidence"
M2_PROJECT_ID=01990000-0000-7000-8000-000000000001
M2_HOST=m2-manual-01990000-0000-7000-8000-000000000002
M2_DATABASE=forge_m2_00000000000000000000000000000001
M2_LANE=openai_api M2_MODEL=synthetic-model M2_ACCOUNT='' M2_SPEND=250000 M2_RUN_WALL=600
M2_IMAGE="localhost/synthetic@sha256:$(printf '%064d' 0)"
M2_ENROLL_FILE="$TEST_ROOT/synthetic-key"
printf '%s\n' 'SYNTHETIC_SECRET_MUST_NOT_PRINT' >"$M2_ENROLL_FILE"
M2_AUTH_SOURCE=$M2_ENROLL_FILE M2_PROXY=http://127.0.0.1:4000 M2_PROXY_MASTER=$M2_ENROLL_FILE
M2_APPROVED=false
if m2_settings </dev/null >"$TEST_ROOT/refused.log" 2>&1; then test_fail 'noninteractive run did not require --yes'; fi
[[ $(jq -r '.lane' "$M2_PRIVATE/config.json") == null ]] || test_fail 'unapproved config was persisted'
M2_APPROVED=true
m2_settings </dev/null >"$TEST_ROOT/settings.log"
! rg -q SYNTHETIC_SECRET_MUST_NOT_PRINT "$M2_PRIVATE/config.json" "$TEST_ROOT/settings.log" || test_fail 'raw secret entered settings/output'
ln -s "$M2_ENROLL_FILE" "$TEST_ROOT/symlink-key"
if m2_private_file "$TEST_ROOT/symlink-key" >/dev/null 2>&1; then test_fail 'symlink credential accepted'; fi
ln "$M2_ENROLL_FILE" "$TEST_ROOT/hardlink-key"
if m2_private_file "$TEST_ROOT/hardlink-key" >/dev/null 2>&1; then test_fail 'hardlink credential accepted'; fi

readonly PIPELINE=01990000-0000-7000-8000-000000000010
readonly VERSION=01990000-0000-7000-8000-000000000011
readonly TASK_A=01990000-0000-7000-8000-000000000020
readonly TASK_B=01990000-0000-7000-8000-000000000021
printf '0\n' >"$TEST_ROOT/employee-count"
printf '0\n' >"$TEST_ROOT/task-count"
printf '0\n' >"$TEST_ROOT/thread-count"
printf '0\n' >"$TEST_ROOT/message-count"
: >"$TEST_ROOT/commands.jsonl"
: >"$TEST_ROOT/profiles.jsonl"

m2_profile_template() {
    printf '%s\n' "$1" >>"$TEST_ROOT/profiles.jsonl"
    jq -c '{employee_id,binding:{synthetic_template:.}}' <<<"$1"
}

m2_command() {
    local name=$1 payload=$2 id kind counter count
    jq -cn --arg name "$name" --argjson payload "$payload" '{name:$name,payload:$payload}' >>"$TEST_ROOT/commands.jsonl"
    case "$name" in
        create_pipeline) id=$PIPELINE; kind=pipeline ;;
        create_employee) counter=employee; kind=employee ;;
        create_task) counter=task; kind=task ;;
        open_employee_thread) counter=thread; kind=employee_thread ;;
        send_employee_message) counter=message; kind=employee_message ;;
        register_project_repository) id=01990000-0000-7000-8000-000000000012; kind=project_repository ;;
        *) id=$M2_PROJECT_ID; kind=project ;;
    esac
    if [[ -n ${counter:-} ]]; then
        read -r count <"$TEST_ROOT/$counter-count"
        count=$((count+1)); printf '%s\n' "$count" >"$TEST_ROOT/$counter-count"
        case "$counter" in
            employee) printf -v id '01990000-0000-7000-8000-%012d' "$((30+count))" ;;
            task) [[ $count != 1 ]] || id=$TASK_A; [[ $count != 2 ]] || id=$TASK_B ;;
            thread) printf -v id '01990000-0000-7000-8000-%012d' "$((40+count))" ;;
            message) printf -v id '01990000-0000-7000-8000-%012d' "$((50+count))" ;;
        esac
    fi
    jq -cn --arg id "$id" --arg kind "$kind" '{status:"applied",resource:{id:$id,kind:$kind}}'
}
m2_read() {
    case "$1" in
        */tasks/*) jq -cn '{revision:1,lifecycle:"ready"}' ;;
        *) jq -cn '{revision:1,execution_gate:"open"}' ;;
    esac
}
m2_pages() {
    case "$1" in
        */pipelines) jq -cn --arg id "$PIPELINE" --arg version "$VERSION" '{items:[{pipeline_id:$id,id:$version,version:1}]}' ;;
        */threads) jq -cn --arg id "$M2_THREAD_A" '{items:[{id:$id,revision:4}]}' ;;
        *) jq -cn '{items:[]}' ;;
    esac
}
m2_bind_session
m2_seed >"$TEST_ROOT/seed.log" 2>"$TEST_ROOT/seed.err"
[[ $(jq -r '.launcher_pid' "$M2_ROOT/session.json") == "$M1_LAUNCHER_PID" ]] || test_fail 'session persisted a transient subshell PID'
[[ $(jq -r '.launcher_token' "$M2_ROOT/session.json") == "$(m1_process_token "$M1_LAUNCHER_PID")" ]] || test_fail 'wrong launcher lifetime'
[[ $M2_TASK_A == "$TASK_A" && $M2_TASK_B == "$TASK_B" && $M2_WRITER != "$M2_REVIEWER" && $M2_REVIEWER != "$M2_QA" ]] || test_fail 'distinct identities'
jq -se 'all(.[];.name!="start_project_execution" and .name!="submit_artifact" and .name!="submit_outcome") and
    ([.[]|select(.name=="disable_employee")]|length)==2 and
    ([.[]|select(.name=="bind_task_git_repository")]|length)==2 and
    all(.[]|select(.name=="bind_task_git_repository");.payload.initial_base=={kind:"unborn",object_format:"sha1"}) and
    any(.[];.name=="amend_employee" and .payload.patch.max_concurrent_runs==3)' "$TEST_ROOT/commands.jsonl" >/dev/null || test_fail 'seed crossed ownership/barrier'
jq -se 'length==3 and all(.[];.lane=="openai_api" and .live_input and .budget.max_spend_microusd==250000)' \
    "$TEST_ROOT/profiles.jsonl" >/dev/null || test_fail 'wrong lane/profile/native contract'
jq -se --arg writer "$M2_WRITER" --arg reviewer "$M2_REVIEWER" --arg qa "$M2_QA" \
    'any(.[];.employee_id==$writer and .access=="read_write") and
     all(.[]|select(.employee_id==$reviewer or .employee_id==$qa);.access=="read_only")' \
    "$TEST_ROOT/profiles.jsonl" >/dev/null || test_fail 'role profile access does not satisfy pinned stage workspace'
jq -e '.stages|length==4' "$SCRIPT_DIR/lib/m2-pipeline.json" >/dev/null || test_fail 'pipeline shape'
jq -e 'any(.transitions[];.from_stage_id=="integrate" and .outcome=="stale_base" and .target=={kind:"stage",stage_id:"work"}) and
    any(.stages[];.id=="review" and .acceptance_policy.independent) and
    any(.stages[];.id=="qa" and .workspace.access=="read_only" and .acceptance_policy==null)' \
    "$SCRIPT_DIR/lib/m2-pipeline.json" >/dev/null || test_fail 'missing stale/review/report-only QA policy'
run=$(jq -cn --arg task "$TASK_A" --arg employee "$M2_WRITER" '{id:"01990000-0000-7000-8000-000000000099",task_id:$task,employee_id:$employee,lease_fencing_token:7,environment_epoch:2}')
m2_go "$M2_THREAD_A" "$run" >"$TEST_ROOT/go.log"
jq -se '.[-1].payload | .expected_thread_revision==4 and .target.kind=="exact_run" and
    .target.fencing_token==7 and .target.environment_epoch==2 and .target.context.stage_visit==1' \
    "$TEST_ROOT/commands.jsonl" >/dev/null || test_fail 'GO lost run fence'
! rg -q SYNTHETIC_SECRET_MUST_NOT_PRINT "$TEST_ROOT/commands.jsonl" "$TEST_ROOT/profiles.jsonl" \
    "$TEST_ROOT/seed.log" "$TEST_ROOT/seed.err" "$M2_ROOT/session.json" || test_fail 'secret leaked'
if m2_assess 0 >/dev/null 2>&1; then test_fail 'missing evidence counted as passed'; fi
# The exact addressed GO needs both an Employee reply and a native receipt.
install -d -m 700 "$M2_ROOT/evidence/threads" "$M2_ROOT/evidence/runs"
m2_record "$(jq -cn --arg a "$M2_THREAD_A" --arg b "$M2_THREAD_B" \
    '{task_thread_ids:[$a,$b],first_writer_runs:["01990000-0000-7000-8000-000000000091","01990000-0000-7000-8000-000000000092"],
      go_message_ids:["01990000-0000-7000-8000-000000000081","01990000-0000-7000-8000-000000000082"]}')"
events='[]'
for index in 0 1; do
    thread=$(jq -r --argjson i "$index" '.task_thread_ids[$i]' "$M2_ROOT/session.json")
    message=$(jq -r --argjson i "$index" '.go_message_ids[$i]' "$M2_ROOT/session.json")
    run_id=$(jq -r --argjson i "$index" '.first_writer_runs[$i]' "$M2_ROOT/session.json")
    m2_json_write "$M2_ROOT/evidence/threads/$thread.json" "$(jq -cn --arg message "$message" --arg writer "$M2_WRITER" \
        '{items:[{reply_to:$message,sender:{kind:"employee",id:$writer},body:"GO"}]}')"
    m2_json_write "$M2_ROOT/evidence/runs/$run_id.json" '{"lease_fencing_token":7,"environment_epoch":2}'
    events=$(jq -cn --argjson old "$events" --arg message "$message" --arg run "$run_id" \
        '$old+[{event_type:"runtime_input_observed",payload:{run_id:$run,source_message_id:$message,outcome:"runtime_accepted",fencing_token:7,environment_epoch:2}}]')
done
m2_json_write "$M2_ROOT/evidence/events.json" "$events"
m2_assess_go || test_fail 'exact GO replies and native receipts rejected'
m2_json_write "$M2_ROOT/evidence/events.json" "$(jq -c '.[0].payload.source_message_id="01990000-0000-7000-8000-000000000088"' <<<"$events")"
if m2_assess_go >/dev/null 2>&1; then test_fail 'READY/other acceptance substituted for GO'; fi
m2_json_write "$M2_ROOT/evidence/events.json" "$(jq -c '.[0].payload.environment_epoch=3' <<<"$events")"
if m2_assess_go >/dev/null 2>&1; then test_fail 'wrong epoch native receipt accepted'; fi
m2_json_write "$M2_ROOT/evidence/events.json" "$events"
m2_json_write "$M2_ROOT/evidence/threads/$M2_THREAD_A.json" '{"items":[]}'
if m2_assess_go >/dev/null 2>&1; then test_fail 'native receipt counted as Employee GO reply'; fi
# A dead observer/expired deadline cannot be hidden by a healthy Task snapshot.
M2_DEADLINE=$SECONDS
if m2_guard >/dev/null 2>&1; then test_fail 'expired guard permitted work'; fi
M2_DEADLINE=$((SECONDS+60)) M1_CORE_PID=999999999 M1_SUPERVISOR_PID=999999998
if m2_guard >/dev/null 2>&1; then test_fail 'absent observer permitted work'; fi
# Never accept incomplete evidence just because a child exit code was zero.
M2_STARTED=true
if m2_finish_report 0 0 passed >"$TEST_ROOT/report.log" 2>&1; then test_fail 'missing full evidence accepted'; fi
jq -e '.outcome=="failed" and .assessment=="failed"' "$M2_ROOT/evidence/report.json" >/dev/null || test_fail 'failed assessment not reported'
M2_STARTED=''
if m2_finish_report 1 0 >"$TEST_ROOT/unverified.log"; then test_fail 'failed preparation returned success'; fi
jq -e '.outcome=="unverified"' "$M2_ROOT/evidence/report.json" >/dev/null || test_fail 'unstarted session misrepresented'
[[ -z $(m2_git --git-dir="$M2_TARGET" for-each-ref --format='%(refname)') ]] || test_fail 'seed generated application commits'
bash "$TEST_DIR/m2-ready-test.sh"
bash "$TEST_DIR/m2-progress-test.sh"
bash "$TEST_DIR/m2-json-test.sh"
bash "$TEST_DIR/m2-report-test.sh"
printf 'm2-launcher-test: passed (real empty Git scaffolding; synthetic transport only)\n'
