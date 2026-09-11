#!/usr/bin/env bash
# Persist public projections and Git object identities, never execute worker code.

m2_events() {
    local response events='[]' cursor=0 next count=0
    while ((count++ < 20)); do
        response=$(m1_cli watch --project-id "$M2_PROJECT_ID" --after "$cursor" 2>/dev/null) || return 1
        response=$(jq -sce --argjson after "$cursor" '
            if all(.[];type=="object" and (.project_sequence|type=="number") and
                (.project_sequence|floor)==.project_sequence and .project_sequence>$after)
                and ([.[].project_sequence]==([.[].project_sequence]|sort|unique)) then .
            else error("invalid event page") end' <<<"$response" 2>/dev/null) \
            || { m2_error 'Некорректная страница событий.'; return 1; }
        [[ $(jq length <<<"$response") != 0 ]] || { printf '%s\n' "$events"; return 0; }
        next=$(jq -er 'map(.project_sequence)|max' <<<"$response") || return 1
        [[ $next =~ ^[1-9][0-9]*$ && $next -gt $cursor ]] || return 1
        events=$(printf '%s\n' "$events" "$response" | jq -cs '.[0]+.[1]') || return 1
        cursor=$next
    done
    m2_error 'Event replay exceeded the bounded evidence window.'
}

m2_collect() {
    local items item id directory path
    install -d -m 700 "$M2_ROOT/evidence/tasks" "$M2_ROOT/evidence/runs" \
        "$M2_ROOT/evidence/reviews" "$M2_ROOT/evidence/integrations" "$M2_ROOT/evidence/threads"
    item=$(m2_read "/v1/projects/$M2_PROJECT_ID") || return 1
    m2_json_write "$M2_ROOT/evidence/project.json" "$item" || return 1
    for directory in tasks runs; do
        items=$(m2_pages "/v1/projects/$M2_PROJECT_ID/$directory") || return 1
        m2_json_write "$M2_ROOT/evidence/$directory.json" "$items" || return 1
        while IFS= read -r id; do
            [[ $id =~ ^[0-9a-f-]{36}$ ]] || return 1
            item=$(m2_read "/v1/projects/$M2_PROJECT_ID/$directory/$id") || return 1
            m2_json_write "$M2_ROOT/evidence/$directory/$id.json" "$item" || return 1
            if [[ $directory == tasks ]]; then
                for path in reviews integrations; do
                    item=$(m2_pages "/v1/projects/$M2_PROJECT_ID/tasks/$id/$path") || return 1
                    m2_json_write "$M2_ROOT/evidence/$path/$id.json" "$item" || return 1
                done
            fi
        done < <(jq -r '.items[].id' <<<"$items")
    done
    while IFS= read -r id; do
        [[ $id =~ ^[0-9a-f-]{36}$ ]] || return 1
        item=$(m2_pages "/v1/projects/$M2_PROJECT_ID/threads/$id/messages") || return 1
        m2_json_write "$M2_ROOT/evidence/threads/$id.json" "$item" || return 1
    done < <(jq -r '.task_thread_ids[]?,.communication_thread_id // empty' "$M2_ROOT/session.json")
    item=$(m2_events) || return 1
    m2_json_write "$M2_ROOT/evidence/events.json" "$item" || return 1
}

m2_target_evidence() {
    local sha tree='[]' mode kind object path line
    sha=$(m2_git --git-dir="$M2_TARGET" rev-parse --verify refs/heads/main 2>/dev/null) || sha=''
    if [[ -n $sha ]]; then
        [[ $sha =~ ^[0-9a-f]{40}$ ]] || return 1
        while IFS= read -r -d '' line; do
            IFS=$'\t' read -r object path <<<"$line"
            read -r mode kind object <<<"$object"
            tree=$(jq -c --arg mode "$mode" --arg kind "$kind" --arg object "$object" --arg path "$path" \
                '. + [{mode:$mode,kind:$kind,object:$object,path:$path}]' <<<"$tree") || return 1
        done < <(m2_git --git-dir="$M2_TARGET" ls-tree -rz --full-tree "$sha")
    fi
    jq -c --arg sha "$sha" '{target_ref:"refs/heads/main",commit:(if $sha=="" then null else $sha end),tree:.}' <<<"$tree"
}

m2_assess() {
    local cleanup=$1 task_files=() review_files=() integration_files=() run_files=() file
    for file in "$M2_ROOT"/evidence/tasks/*.json; do [[ ! -f $file ]] || task_files+=("$file"); done
    for file in "$M2_ROOT"/evidence/reviews/*.json; do [[ ! -f $file ]] || review_files+=("$file"); done
    for file in "$M2_ROOT"/evidence/integrations/*.json; do [[ ! -f $file ]] || integration_files+=("$file"); done
    for file in "$M2_ROOT"/evidence/runs/*.json; do [[ ! -f $file ]] || run_files+=("$file"); done
    ((${#task_files[@]}==2 && ${#review_files[@]}==2 && ${#integration_files[@]}==2)) || return 1
    jq -e -s 'length==2 and all(.[];.lifecycle=="done" and
        any(.artifacts[];.kind=="work_report") and any(.artifacts[];.kind=="review_report") and
        any(.artifacts[];.kind=="qa_report") and any(.artifacts[];.kind=="integration_result"))' "${task_files[@]}" >/dev/null || return 1
    jq -e -s --arg writer "$M2_WRITER" '[.[].items[]] as $r | ($r|length)>=3 and
        all(.[];any(.items[];.verdict=="accepted")) and
        all($r[];.employee_id!=$writer) and
        any(.[];([.items[]|select(.verdict=="accepted")|.candidate.commit]|unique|length)>=2)' "${review_files[@]}" >/dev/null || return 1
    jq -e -s '[.[].items[]] as $ops | any($ops[];.result_code=="stale_base") and
        all(.[];any(.items[];.state=="completed" and (.result_code=="applied" or .result_code=="no_changes")))' \
        "${integration_files[@]}" >/dev/null || return 1
    jq -e '.owned_containers_running and (.runs|length)==2' "$M2_ROOT/evidence/initial-overlap.json" >/dev/null || return 1
    jq -e '.owned_containers_running and (.runs|length)==3 and
        ([.runs[].employee_id]|unique|length)==1 and
        any(.runs[];.assignment.purpose=="communication" and .task_id==null)' \
        "$M2_ROOT/evidence/communication-overlap.json" >/dev/null || return 1
    jq -e '.target_unborn and .first_writers_stopped and all(.tasks[];.current_stage_id=="review")' \
        "$M2_ROOT/evidence/first-candidates.json" >/dev/null || return 1
    jq -e '.commit!=null and (.tree|length)==2 and
        any(.tree[];.path=="README.md" and .kind=="blob" and (.mode=="100644" or .mode=="100755")) and
        any(.tree[];.path=="normalize-lines.sh" and .kind=="blob" and .mode=="100755")' \
        "$M2_ROOT/evidence/target.json" >/dev/null || return 1
    jq -e -s --slurpfile target "$M2_ROOT/evidence/target.json" \
        'any(.[].items[];.state=="completed" and .result_code=="applied" and .prepared_merge.merge_commit==$target[0].commit)' \
        "${integration_files[@]}" >/dev/null || return 1
    ((${#run_files[@]}>0)) || return 1
    jq -e -s 'all(.[];.observed_state=="stopped" and .diagnostics.runtime_report!=null)' "${run_files[@]}" >/dev/null || return 1
    jq -e --slurpfile session "$M2_ROOT/session.json" \
        'any(.[];.event_type=="communication_completed" and .payload.run_id==$session[0].communication_run_id)' \
        "$M2_ROOT/evidence/events.json" >/dev/null || return 1
    m2_assess_go || return 1
    jq -e '.ready_overlap and .communication_answered and (.go_message_ids|length)==2' "$M2_ROOT/session.json" >/dev/null || return 1
    [[ $cleanup == 0 ]]
}

m2_assess_go() {
    local index thread message run detail
    jq -e '(.first_writer_runs|length)==2 and (.task_thread_ids|length)==2 and
        (.go_message_ids|length)==2 and (.go_message_ids|unique|length)==2' "$M2_ROOT/session.json" >/dev/null || return 1
    for index in 0 1; do
        thread=$(jq -er --argjson i "$index" '.task_thread_ids[$i]' "$M2_ROOT/session.json") || return 1
        message=$(jq -er --argjson i "$index" '.go_message_ids[$i]' "$M2_ROOT/session.json") || return 1
        run=$(jq -er --argjson i "$index" '.first_writer_runs[$i]' "$M2_ROOT/session.json") || return 1
        [[ $thread =~ ^[0-9a-f-]{36}$ && $message =~ ^[0-9a-f-]{36}$ && $run =~ ^[0-9a-f-]{36}$ ]] || return 1
        detail="$M2_ROOT/evidence/runs/$run.json"
        jq -e --arg message "$message" --arg employee "$M2_WRITER" \
            'any(.items[];.reply_to==$message and .sender.kind=="employee" and .sender.id==$employee and
                (.body|gsub("^\\s+|\\s+$";""))=="GO")' "$M2_ROOT/evidence/threads/$thread.json" >/dev/null || return 1
        # READY acceptance or another Run epoch cannot stand in for the GO transport receipt.
        jq -e --arg message "$message" --arg run "$run" --slurpfile detail "$detail" \
            'any(.[];.event_type=="runtime_input_observed" and .payload.run_id==$run and
                .payload.source_message_id==$message and .payload.outcome=="runtime_accepted" and
                .payload.fencing_token==$detail[0].lease_fencing_token and
                .payload.environment_epoch==$detail[0].environment_epoch)' \
            "$M2_ROOT/evidence/events.json" >/dev/null || return 1
    done
}

m2_finish_report() {
    local workflow_status=$1 cleanup=$2 evidence_collection=${3:-unverified}
    local status=$1 outcome=unverified workflow=unverified assessment=unverified target report
    case "$evidence_collection" in passed|failed|unverified) ;; *) return 1 ;; esac
    install -d -m 700 "$M2_ROOT/evidence" || return 1
    if ! target=$(m2_target_evidence); then
        target='{"commit":null,"tree":[]}'
        evidence_collection=failed
    fi
    m2_json_write "$M2_ROOT/evidence/target.json" "$target" || return 1
    if ((cleanup != 0)) || [[ $evidence_collection == failed ]]; then
        ((status != 0)) || status=1
    fi
    if [[ -n ${M2_STARTED:-} ]]; then
        outcome=failed workflow=failed
        ((workflow_status != 0)) || workflow=passed
        if ((workflow_status==0 && cleanup==0)) && [[ $evidence_collection == passed ]]; then
            if m2_assess "$cleanup" >"$M2_ROOT/assessment.log" 2>&1; then outcome=passed; assessment=passed
            else assessment=failed; status=1; fi
        else
            ((status != 0)) || status=1
        fi
    fi
    report=$(printf '%s\n' "$target" | jq -c --arg outcome "$outcome" --arg assessment "$assessment" \
        --arg workflow "$workflow" --arg evidence "$evidence_collection" \
        --argjson workflow_exit "$workflow_status" --argjson exit "$status" --argjson cleanup "$cleanup" \
        --arg lane "${M2_LANE:-}" --arg model "${M2_MODEL:-}" \
        '. as $target | {scenario:"m2_live_git_workflow",outcome:$outcome,assessment:$assessment,exit_status:$exit,
          workflow:$workflow,workflow_exit_status:$workflow_exit,evidence_collection:$evidence,
          cleanup:(if $cleanup==0 then "passed" else "failed" end),lane:$lane,model:$model,target:$target,
          note:"Live claims apply only to this selected lane. Semantic reports are Employee claims, not a host test verdict. Other provider lanes and pin/file exercises remain unverified."}') || return 1
    m2_json_write "$M2_ROOT/evidence/report.json" "$report" || return 1
    m2_record "$(jq -cn --arg outcome "$outcome" '{outcome:$outcome,phase:"closed"}')" || return 1
    printf 'M2 outcome: %s; evidence collection: %s; cleanup: %s\n' \
        "$outcome" "$evidence_collection" "$(if ((cleanup==0)); then printf passed; else printf failed; fi)"
    return "$status"
}

m2_status_projection() {
    printf '%s\n' "$1" "$2" | jq -s --arg root "$M2_ROOT" \
        '{session:$root,project:.[0],tasks:[.[1].items[]|{id,key,lifecycle,current_stage_id}]}'
}

m2_status() {
    local project tasks
    m2_load_session || return 1
    if [[ -S $M2_ROOT/api.sock ]] && project=$(m2_read "/v1/projects/$M2_PROJECT_ID" 2>/dev/null); then
        tasks=$(m2_pages "/v1/projects/$M2_PROJECT_ID/tasks") || return 1
        m2_status_projection "$project" "$tasks"
    else
        jq '{root,project_id,phase,outcome,task_ids,lane,model}' "$M2_ROOT/session.json"
        printf 'Core недоступен; показано последнее сохранённое состояние.\n' >&2
    fi
}

m2_evidence_command() {
    m2_load_session || return 1
    if [[ -S $M2_ROOT/api.sock ]]; then
        m2_collect || { m2_error 'API evidence не обновлено; сохранённые данные остаются на месте.'; return 1; }
    fi
    printf 'Evidence: %s/evidence/\nSession: %s/session.json\nTarget: %s\n' "$M2_ROOT" "$M2_ROOT" "$M2_TARGET"
    if [[ -f $M2_ROOT/evidence/report.json ]]; then
        jq '{scenario,outcome,assessment,cleanup,evidence_collection:(.evidence_collection // "unverified"),
            workflow:(.workflow // "unverified"),workflow_exit_status,exit_status,lane,model,target_commit:.target.commit}' \
            "$M2_ROOT/evidence/report.json"
    else
        printf 'unverified: сценарий ещё не завершён, итогового report.json нет.\n'
    fi
}
