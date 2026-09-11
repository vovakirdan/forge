#!/usr/bin/env bash
# Observe canonical facts; only Employees write files, artifacts and verdicts.

m2_guard() {
    local state runs
    ((SECONDS < M2_DEADLINE)) || { m2_error 'Истёк общий срок сценария.'; return 1; }
    m1_process_token "$M1_CORE_PID" >/dev/null && m1_process_token "$M1_SUPERVISOR_PID" >/dev/null \
        || { m2_error 'Core/Supervisor остановился.'; return 1; }
    state=$(m2_read "/v1/projects/$M2_PROJECT_ID") || return 1
    [[ $(jq -r '.execution_gate' <<<"$state") == open ]] || { m2_error 'Project остановлен.'; return 1; }
    runs=$(m2_pages "/v1/projects/$M2_PROJECT_ID/runs") || return 1
    [[ $(jq '.items | length' <<<"$runs") -lt $M2_RUN_GUARD ]] \
        || { m2_error 'Сработал observed Run guard. Автоматических платных повторов не будет.'; return 1; }
}

m2_phase() {
    m2_record "$(jq -cn --arg phase "$1" '{phase:$phase}')" || return 1
    printf 'M2: %s\n' "$2"
}

m2_answered() {
    local thread=$1 message=$2 exact=${3:-} messages
    messages=$(m2_pages "/v1/projects/$M2_PROJECT_ID/threads/$thread/messages") || return 1
    jq -e --arg message "$message" --arg exact "$exact" --arg employee "$M2_WRITER" '
        any(.items[]; .reply_to==$message and .sender.kind=="employee" and .sender.id==$employee and
            ($exact=="" or (.body | gsub("^\\s+|\\s+$";""))==$exact))' <<<"$messages" >/dev/null
}

m2_first_runs() {
    local runs=${1:-}
    [[ -n $runs ]] || runs=$(m2_pages "/v1/projects/$M2_PROJECT_ID/runs") || return 1
    M2_RUN_A=$(jq -ce --arg task "$M2_TASK_A" '[.items[] | select(.task_id==$task and .stage_id=="work")] | select(length==1) | .[0] | select(.observed_state=="running")' <<<"$runs") || return 1
    M2_RUN_B=$(jq -ce --arg task "$M2_TASK_B" '[.items[] | select(.task_id==$task and .stage_id=="work")] | select(length==1) | .[0] | select(.observed_state=="running")' <<<"$runs") || return 1
    printf '%s\n' "$M2_RUN_A" "$M2_RUN_B" | jq -es --arg writer "$M2_WRITER" \
        '.[0].id!=.[1].id and all(.[];.employee_id==$writer)' >/dev/null
}

# Only the initial READY barrier uses this guard. Historical failed Runs are
# normal during later rework and must not fail the general workflow guard.
m2_initial_writer_health() {
    local runs=$1 task_id task run run_id detail summary
    for task_id in "$M2_TASK_A" "$M2_TASK_B"; do
        task=$(m2_read "/v1/projects/$M2_PROJECT_ID/tasks/$task_id") || return 1
        run=$(jq -c --arg task "$task_id" --arg writer "$M2_WRITER" '
            [.items[] | select(.task_id==$task and .stage_id=="work" and .employee_id==$writer)] |
            sort_by(.lease_fencing_token) | .[0] // null' <<<"$runs") || return 1
        if ! printf '%s\n' "$task" "$run" | jq -es '.[0] as $task | .[1] as $run |
            ($task.lifecycle=="waiting" or $task.lifecycle=="cancelled" or $task.lifecycle=="done") or
            ($run!=null and (
                (["failed","stopped","lost","stopping"] | index($run.observed_state))!=null or
                (["failed","stopped","stop_requested","force_stop_requested"] | index($run.desired_state))!=null or
                ($run.observed_state=="unknown" and $run.desired_state!="provision_requested")))' >/dev/null; then
            continue
        fi
        run_id=$(jq -r '.id // ""' <<<"$run") || return 1
        detail='{}'
        if [[ $run_id =~ ^[0-9a-f-]{36}$ ]]; then
            detail=$(m2_read "/v1/projects/$M2_PROJECT_ID/runs/$run_id") || detail='{}'
        fi
        # Never print free-form wait details, provider messages, prompts or logs.
        summary=$(printf '%s\n' "$task" "$run" "$detail" | jq -sr --arg task_id "$task_id" '
            def code: if type=="string" and test("^[a-z][a-z0-9_]{0,63}$") then . else "unavailable" end;
            def identity: if type=="string" and test("^[0-9a-f-]{36}$") then . else "not_created" end;
            .[0] as $task | .[1] as $run | .[2] as $detail |
            "Task=\($task_id|identity), lifecycle=\($task.lifecycle|code), " +
            "Run=\($run.id|identity), desired=\(($detail.desired_state//$run.desired_state)|code), " +
            "observed=\(($detail.observed_state//$run.observed_state)|code), " +
            "reason_code=unavailable, " +
            "failure_kind=\($detail.diagnostics.runtime_report.failure|code), " +
            "incident_kind=\($detail.diagnostics.incidents[0].kind|code), wait_kind=\($task.wait_conditions[0].kind|code)"') || return 1
        m2_error "READY прерван: $summary. Это публичные состояния, не полная причина отказа; подробности: $M2_ROOT/supervisor.log. Автоматического повторного Run не будет."
        return 1
    done
}

m2_container() {
    jq -er '"forge-run-" + .id + "-" + (.lease_fencing_token|tostring) + "-" + (.environment_epoch|tostring)' <<<"$1"
}

m2_prove_isolation() {
    local a b mounts_a mounts_b evidence
    a=$(m2_container "$M2_RUN_A") && b=$(m2_container "$M2_RUN_B") || return 1
    [[ $(m1_container_state "$a") == running && $(m1_container_state "$b") == running ]] || return 1
    mounts_a=$(m1_podman inspect --format '{{json .Mounts}}' "$a") || return 1
    mounts_b=$(m1_podman inspect --format '{{json .Mounts}}' "$b") || return 1
    printf '%s\n' "$mounts_a" "$mounts_b" | jq -es --arg root "$M1_EXEC/surfaces/" '
        .[0] as $a | .[1] as $b |
        [$a[]|select(.Destination=="/workspace")|.Source] as $left |
        [$b[]|select(.Destination=="/workspace")|.Source] as $right |
        ($left|length)==1 and ($right|length)==1 and $left[0]!=$right[0] and
        ($left[0]|startswith($root)) and ($right[0]|startswith($root))' >/dev/null || return 1
    evidence=$(printf '%s\n' "$M2_RUN_A" "$M2_RUN_B" "$mounts_a" "$mounts_b" | jq -cs \
        '{runs:.[0:2],mounts:.[2:4],owned_containers_running:true}') || return 1
    m2_json_write "$M2_ROOT/evidence/initial-overlap.json" "$evidence"
}

m2_ready_barrier() {
    local deadline=$((SECONDS + 180)) runs
    m2_phase ready 'Жду READY двух writers, без редактирования файлов.'
    while ((SECONDS < deadline)); do
        m2_guard || return 1
        runs=$(m2_pages "/v1/projects/$M2_PROJECT_ID/runs") || return 1
        m2_initial_writer_health "$runs" || return 1
        if m2_first_runs "$runs" && m2_answered "$M2_THREAD_A" "$M2_READY_A" READY \
            && m2_answered "$M2_THREAD_B" "$M2_READY_B" READY && m2_prove_isolation; then
            m2_record "$(jq -cn --arg a "$(jq -r '.id' <<<"$M2_RUN_A")" --arg b "$(jq -r '.id' <<<"$M2_RUN_B")" '{first_writer_runs:[$a,$b],ready_overlap:true}')"
            return 0
        fi
        sleep 0.3
    done
    m2_error 'Два READY и их физический overlap не подтверждены за 180 секунд.'
}

m2_communication() {
    local thread message response run='' observed=false deadline=$((SECONDS + 120)) container evidence
    m2_phase communication 'Общий Inbox вопрос: отдельный третий Run того же writer.'
    thread=$(m2_open_thread "$M2_WRITER") || return 1
    message=$(m2_send "$thread" 1 '{"kind":"inbox"}' \
        'This is a taskless Communication assignment while two Task workers await GO. Read the board through Forge tools and briefly report the two live Tasks and stages. Reply to this source message, then call forge_complete_communication. Do not edit files, inspect another workspace, complete a Task, or wait for GO.' question) || return 1
    m2_record "$(jq -cn --arg thread "$thread" --arg message "$message" '{communication_thread_id:$thread,communication_message_id:$message}')"
    while ((SECONDS < deadline)); do
        m2_guard || return 1
        response=$(m2_pages "/v1/projects/$M2_PROJECT_ID/runs") || return 1
        run=$(jq -c --arg thread "$thread" '[.items[]|select(.assignment.purpose=="communication" and .assignment.owner.thread_id==$thread)] | if length==1 then .[0] else null end' <<<"$response") || return 1
        if [[ $run != null ]]; then
            if [[ $observed == false ]] && m2_first_runs && [[ $(jq -r '.observed_state' <<<"$run") == running ]]; then
                container=$(m2_container "$run") || return 1
                if [[ $(m1_container_state "$container") == running ]] && m2_prove_isolation; then
                    observed=true
                    evidence=$(printf '%s\n' "$M2_RUN_A" "$M2_RUN_B" "$run" | jq -cs '{runs:.,owned_containers_running:true}') || return 1
                    m2_json_write "$M2_ROOT/evidence/communication-overlap.json" "$evidence" || return 1
                fi
            fi
            if m2_answered "$thread" "$message"; then
                [[ $observed == true ]] || { m2_error 'Ответ получен, но физический overlap трёх Runs не зафиксирован; не объявляем passed.'; return 1; }
                m2_record "$(jq -cn --arg run "$(jq -r '.id' <<<"$run")" '{communication_run_id:$run,communication_answered:true}')"
                return 0
            fi
        fi
        sleep 0.2
    done
    m2_error 'Communication не ответил за 120 секунд.'
}

m2_go() {
    local thread=$1 run=$2 threads revision target message
    threads=$(m2_pages "/v1/projects/$M2_PROJECT_ID/employees/$M2_WRITER/threads") || return 1
    revision=$(jq -er --arg id "$thread" '.items[]|select(.id==$id)|.revision' <<<"$threads") || return 1
    target=$(jq -c --arg pipeline "$M2_PIPELINE" '{kind:"exact_run",context:{task_id:.task_id,pipeline_version_id:$pipeline,stage_id:"work",stage_visit:1},run_id:.id,fencing_token:.lease_fencing_token,environment_epoch:.environment_epoch}' <<<"$run") || return 1
    message=$(m2_send "$thread" "$revision" "$target" 'GO. Reply GO through forge_reply_instruction to this exact message, then implement only your own Task file, verify and commit. Submit actual work_report and completed Git candidate outcome. Do not wait for another instruction.') || return 1
    printf '%s\n' "$message"
}

m2_candidate_barrier() {
    local task a b detail id evidence
    m2_phase candidates 'GO доставлен адресно. Жду принятия первых candidates и остановки writers.'
    while :; do
        m2_guard || return 1
        a=$(m2_read "/v1/projects/$M2_PROJECT_ID/tasks/$M2_TASK_A") || return 1
        b=$(m2_read "/v1/projects/$M2_PROJECT_ID/tasks/$M2_TASK_B") || return 1
        for task in "$a" "$b"; do
            [[ $(jq -r '.lifecycle' <<<"$task") != waiting ]] || { m2_error 'Writer требует решения; смотри wait_conditions.'; return 1; }
        done
        if printf '%s\n' "$a" "$b" | jq -es 'all(.[];.current_stage_id=="review")' >/dev/null; then
            for id in "$(jq -r '.id' <<<"$M2_RUN_A")" "$(jq -r '.id' <<<"$M2_RUN_B")"; do
                detail=$(m2_read "/v1/projects/$M2_PROJECT_ID/runs/$id") || return 1
                jq -e '.observed_state=="stopped" and .diagnostics.runtime_report!=null' <<<"$detail" >/dev/null || break
            done
            if [[ $id == "$(jq -r '.id' <<<"$M2_RUN_B")" ]] && jq -e '.observed_state=="stopped" and .diagnostics.runtime_report!=null' <<<"$detail" >/dev/null; then
                [[ -z $(m2_git --git-dir="$M2_TARGET" for-each-ref --format='%(refname)') ]] \
                    || { m2_error 'Target изменился до открытия review barrier.'; return 1; }
                evidence=$(printf '%s\n' "$a" "$b" | jq -cs '{tasks:.,target_unborn:true,first_writers_stopped:true}') || return 1
                m2_json_write "$M2_ROOT/evidence/first-candidates.json" "$evidence" || return 1
                return 0
            fi
        fi
        sleep 1
    done
}

m2_progress() {
    local tasks=$1 runs=$2 remaining=$3
    # State counts are observations, not proof of activity or a scheduling cause.
    printf '%s\n' "$tasks" "$runs" | jq -sr --argjson remaining "$remaining" '
        .[0] as $tasks | .[1] as $runs |
        [$runs.items[] | select(.observed_state!="stopped" and .observed_state!="failed")] as $active |
        [$tasks.items[] | select(.lifecycle=="ready" or .lifecycle=="in_progress")] as $unfinished |
        [$unfinished[] | select(.current_stage_id=="work" or .current_stage_id=="review" or .current_stage_id=="qa") |
            . as $task | select(any($active[]; .task_id==$task.id and .stage_id==$task.current_stage_id)|not)] | length as $awaiting |
        "M2: до общего таймаута \([$remaining,0]|max)s; " +
        "Runs: ожидают запуска=\([$active[]|select(.observed_state=="unknown" and .desired_state=="provision_requested")]|length), " +
        "provisioning=\([$active[]|select(.observed_state=="provisioning")]|length), " +
        "running=\([$active[]|select(.observed_state=="running")]|length), " +
        "stopping=\([$active[]|select(.observed_state=="stopping")]|length), " +
        "неопределённых=\([$active[]|select(.observed_state!="provisioning" and .observed_state!="running" and .observed_state!="stopping" and
            (.observed_state!="unknown" or .desired_state!="provision_requested"))]|length). " +
        "Task без активного Run, ожидают исполнения=\($awaiting); " +
        "на системной стадии=\([$unfinished[]|select(.current_stage_id=="integrate")]|length). " +
        "Причина ожидания из этой сводки не определяется."'
}

m2_wait_done() {
    local tasks previous='' summary runs next_progress=$SECONDS
    while :; do
        m2_guard || return 1
        tasks=$(m2_pages "/v1/projects/$M2_PROJECT_ID/tasks") || return 1
        summary=$(jq -cr '[.items[]|{id,key,lifecycle,current_stage_id}]' <<<"$tasks") || return 1
        if [[ $summary != "$previous" ]]; then printf '%s\n' "$summary"; previous=$summary; fi
        if jq -e '(.items|length)==2 and all(.items[];.lifecycle=="done")' <<<"$tasks" >/dev/null; then return 0; fi
        if jq -e 'any(.items[];.lifecycle=="waiting" or .lifecycle=="cancelled")' <<<"$tasks" >/dev/null; then
            m2_error 'Task остановилась в waiting/cancelled. Сценарий не подменяет решение менеджера.'; return 1
        fi
        if ((SECONDS >= next_progress)); then
            runs=$(m2_pages "/v1/projects/$M2_PROJECT_ID/runs") || return 1
            m2_progress "$tasks" "$runs" "$((M2_DEADLINE - SECONDS))" || return 1
            next_progress=$((SECONDS + 30))
        fi
        sleep 1
    done
}

m2_workflow() {
    local go_a go_b employee
    M2_DEADLINE=$((SECONDS + M2_SESSION_WALL))
    M2_STARTED=true
    m2_command start_project_execution '{"reason":"Operator approved bounded real M2 Git workflow"}' >/dev/null
    m2_ready_barrier
    m2_communication
    go_a=$(m2_go "$M2_THREAD_A" "$M2_RUN_A")
    go_b=$(m2_go "$M2_THREAD_B" "$M2_RUN_B")
    m2_record "$(jq -cn --arg a "$go_a" --arg b "$go_b" '{go_message_ids:[$a,$b]}')"
    m2_candidate_barrier
    for employee in "$M2_REVIEWER" "$M2_QA"; do
        m2_command enable_employee "$(jq -cn --arg id "$employee" '{employee_id:$id,expected_employee_revision:2}')" >/dev/null
    done
    m2_phase delivery 'Review → QA → Integration; stale_base возвращает ту же Task в work.'
    m2_wait_done
    m2_record '{"phase":"done_observed"}'
}
