#!/usr/bin/env bash

m3_guard() {
    local runs
    ((SECONDS<M3_DEADLINE)) || { m3_error 'Истёк общий срок сценария.'; return 1; }
    [[ $M3_MODE == run ]] || return 0
    runs=$(m1_read "/v1/projects/$M1_PROJECT_ID/runs?limit=100")
    jq -e '.next_cursor==null and (.items|length)<6' <<<"$runs" >/dev/null \
        || { m3_error 'Достигнут observed Run guard; остановка.'; return 1; }
}

m3_wait_index() {
    local page=$1 output=$2 deadline=$((SECONDS+120))
    while ((SECONDS<deadline)); do
        m3_guard
        m1_read "/v1/projects/$M1_PROJECT_ID/memory/search?query=violet%20compass&limit=10" >"$output"
        if jq -e --arg page "$page" '.mode=="indexed" and any(.results[]; .document.kind=="knowledge_page" and .document.record.id==$page)' "$output" >/dev/null; then return 0; fi
        sleep 2
    done
    m3_error 'Strict index/query не подтверждён за 120s.'
}

m3_index_scenario() {
    M3_PHASE=index
    printf 'M3: canonical policy → strict index → restart/readback → outage/fallback → catch-up.\n'
    M3_POLICY=$(m3_publish 'M3 violet compass policy' 'The violet compass policy: change only files in your assigned work surface. Report actual observations, never infer that tests passed merely because a process exited.')
    m3_wait_index "$M3_POLICY" "$M1_ROOT/evidence/index-before-restart.json"
    m3_index_control restart --time=10
    m3_wait_index "$M3_POLICY" "$M1_ROOT/evidence/index-after-restart.json"
    m3_index_control stop --time=10
    m1_read "/v1/projects/$M1_PROJECT_ID/memory/search?query=violet%20compass&limit=10" >"$M1_ROOT/evidence/index-outage.json"
    jq -e '.mode=="canonical_fallback" and .degradation=="index_unavailable" and (.results|length)>0' "$M1_ROOT/evidence/index-outage.json" >/dev/null
    M3_OUTAGE_POLICY=$(m3_publish 'M3 offline violet compass addition' 'The violet compass knowledge remains canonical while the retrieval service is offline. A summary attributes employee reports; it does not verify their truth.')
    m1_read "/v1/projects/$M1_PROJECT_ID/knowledge/$M3_OUTAGE_POLICY" >"$M1_ROOT/evidence/authored-during-outage.json"
    m1_read "/v1/projects/$M1_PROJECT_ID/memory/status" >"$M1_ROOT/evidence/backlog-during-outage.json"
    jq -e '.pending>0' "$M1_ROOT/evidence/backlog-during-outage.json" >/dev/null
    m3_index_control start
    m3_wait_index "$M3_OUTAGE_POLICY" "$M1_ROOT/evidence/index-caught-up.json"
}

m3_wait_onboarding() {
    local previous='' state last_log=0
    while :; do
        m3_guard
        m1_read "/v1/projects/$M1_PROJECT_ID/employees/$M1_EMPLOYEE_ID/onboarding" >"$M1_ROOT/evidence/onboarding-current.json"
        state=$(jq -er '.state' "$M1_ROOT/evidence/onboarding-current.json")
        if [[ $state != "$previous" ]] || ((SECONDS-last_log>=30)); then printf 'M3: onboarding=%s, осталось %ss.\n' "$state" "$((M3_DEADLINE-SECONDS))"; previous=$state; last_log=$SECONDS; fi
        case "$state" in completed) return 0 ;; skipped|legacy_bypass|failed) m3_error 'Onboarding не завершён реальным accepted result.'; return 1 ;; esac
        m3_check_jobs
        sleep 2
    done
}

m3_check_jobs() {
    m1_read "/v1/projects/$M1_PROJECT_ID/system-jobs" >"$M1_ROOT/evidence/jobs-current.json"
    jq -e 'all(.jobs[]; .state!="held" and .state!="cancelled")' "$M1_ROOT/evidence/jobs-current.json" >/dev/null \
        || { m3_error 'SystemJob требует решения; смотри jobs-current.json.'; return 1; }
}

m3_request_summary() {
    m1_command request_task_summary "$(jq -cn --arg task "$M1_TASK_ID" '{task_id:$task}')" >/dev/null
}

m3_employee_scenario() {
    M3_PHASE=onboarding
    m3_seed_employee
    printf 'M3: pending Employee → ownerless onboarding.\n'
    m1_start_project
    m3_wait_onboarding
    m1_read "/v1/projects/$M1_PROJECT_ID/memory?employee_id=$M1_EMPLOYEE_ID&limit=100" >"$M1_ROOT/evidence/memory-after-onboarding.json"
    jq -e --arg employee "$M1_EMPLOYEE_ID" 'any(.items[]; .subject.kind=="employee_memory_entry" and .subject.employee_id==$employee and .subject.task_id==null)' \
        "$M1_ROOT/evidence/memory-after-onboarding.json" >/dev/null
    M3_PHASE=task
    printf 'M3: обычный Task Run с каноническим контекстом и памятью.\n'
    m3_seed_task
    local state previous='' last_log=0
    while :; do
        m3_guard
        m1_read "/v1/projects/$M1_PROJECT_ID/tasks/$M1_TASK_ID" >"$M1_ROOT/evidence/task.json"
        state=$(jq -er '.lifecycle' "$M1_ROOT/evidence/task.json")
        if [[ $state != "$previous" ]] || ((SECONDS-last_log>=30)); then printf 'M3: Task=%s, осталось %ss.\n' "$state" "$((M3_DEADLINE-SECONDS))"; previous=$state; last_log=$SECONDS; fi
        case "$state" in done) break ;; cancelled|waiting) m3_error 'Task не завершена; смотри evidence/task.json.'; return 1 ;; esac
        m3_check_jobs
        sleep 2
    done
    M3_PHASE=summary
    m3_request_summary
    printf 'M3: ownerless summary → source-linked memory.\n'
    last_log=0
    while :; do
        m3_guard
        m3_check_jobs
        m1_read "/v1/projects/$M1_PROJECT_ID/memory?employee_id=$M1_EMPLOYEE_ID&limit=100" >"$M1_ROOT/evidence/memory-final.json"
        if jq -e --arg task "$M1_TASK_ID" 'any(.jobs[]; .kind=="summarization" and .source_task_id==$task and .state=="completed")' "$M1_ROOT/evidence/jobs-current.json" >/dev/null \
            && jq -e --arg task "$M1_TASK_ID" 'any(.items[]; .subject.kind=="task_summary" and .subject.task_id==$task and .coverage.last_event_sequence>0 and (.source_refs|length)>0)' "$M1_ROOT/evidence/memory-final.json" >/dev/null; then break; fi
        if ((SECONDS-last_log>=30)); then printf 'M3: жду принятого summary, осталось %ss.\n' "$((M3_DEADLINE-SECONDS))"; last_log=$SECONDS; fi
        sleep 2
    done
    m1_read "/v1/projects/$M1_PROJECT_ID/memory/search?query=normalize&employee_id=$M1_EMPLOYEE_ID&limit=10" >"$M1_ROOT/evidence/memory-query-final.json"
}

m3_stop_proof() {
    local before after
    M3_PHASE=stopping
    m1_stop_project
    before=$(m1_read "/v1/projects/$M1_PROJECT_ID/runs?limit=100" | jq -er '.items|length')
    if [[ -n ${M1_TASK_ID:-} ]]; then
        m3_request_summary
    fi
    sleep 6
    after=$(m1_read "/v1/projects/$M1_PROJECT_ID/runs?limit=100" | jq -er '.items|length')
    [[ $before == "$after" ]] || { m3_error 'После project stop появился новый Run.'; return 1; }
    jq -n --argjson before "$before" --argjson after "$after" '{run_count_before:$before,run_count_after:$after,no_dispatch_while_stopped:($before==$after),observation_seconds:6}' >"$M1_ROOT/evidence/stop-proof.json"
}

m3_collect() {
    local id postgres
    # EXIT calls this in an OR-list: Bash disables errexit throughout the function.
    # Every evidence failure must therefore propagate explicitly.
    m1_read "/v1/projects/$M1_PROJECT_ID/runs?limit=100" >"$M1_ROOT/evidence/runs.json" || return 1
    m1_read "/v1/projects/$M1_PROJECT_ID/system-jobs" >"$M1_ROOT/evidence/system-jobs.json" || return 1
    m1_read "/v1/projects/$M1_PROJECT_ID/memory/status" >"$M1_ROOT/evidence/index-status.json" || return 1
    jq -e 'type=="object" and (.items|type)=="array" and .next_cursor==null' "$M1_ROOT/evidence/runs.json" >/dev/null || return 1
    jq -e 'type=="object" and (.jobs|type)=="array"' "$M1_ROOT/evidence/system-jobs.json" >/dev/null || return 1
    jq -e 'type=="object" and (.configured|type)=="boolean"' "$M1_ROOT/evidence/index-status.json" >/dev/null || return 1
    jq -r '.items[].id' "$M1_ROOT/evidence/runs.json" >"$M1_ROOT/evidence/run-ids.txt" || return 1
    while IFS= read -r id; do
        [[ $id =~ ^[0-9a-f-]{36}$ ]] || return 1
        m1_read "/v1/projects/$M1_PROJECT_ID/runs/$id" >"$M1_ROOT/evidence/run-$id.json" || return 1
        jq -e 'type=="object"' "$M1_ROOT/evidence/run-$id.json" >/dev/null || return 1
    done <"$M1_ROOT/evidence/run-ids.txt"
    postgres=$(service_container_id postgres) || return 1
    # This database belongs to this session. Read-only evidence keeps full pinned
    # Run/context/source envelopes in files, never JSON-in-argv or terminal output.
    timeout --kill-after=1 15 podman exec "$postgres" psql --no-psqlrc --tuples-only --no-align \
        --username "$FORGE_POSTGRES_USER" --dbname "$M1_DATABASE" \
        --command "SELECT jsonb_build_object('run_id',id,'purpose',purpose,'task_id',task_id,'employee_id',employee_id,'system_job_attempt_id',system_job_attempt_id,'run_spec',run_spec,'context_manifest',context_manifest) FROM runs ORDER BY id" \
        >"$M1_ROOT/evidence/pinned-contexts.jsonl" 2>"$M1_ROOT/evidence/read-error.log" || return 1
    jq -s . "$M1_ROOT/evidence/pinned-contexts.jsonl" >"$M1_ROOT/evidence/pinned-contexts.json" || return 1
    if [[ $M3_MODE == run && $M3_PHASE == passed ]]; then
        jq -e 'any(.[]; .purpose=="system_job" and .task_id==null and .employee_id==null and .run_spec.assignment.kind=="onboarding") and
            any(.[]; .purpose=="system_job" and .task_id==null and .employee_id==null and .run_spec.assignment.kind=="summarization") and
            any(.[]; .purpose=="task_stage" and .task_id!=null and .employee_id!=null)' "$M1_ROOT/evidence/pinned-contexts.json" >/dev/null || return 1
    fi
    return 0
}
