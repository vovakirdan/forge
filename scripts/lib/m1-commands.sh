#!/usr/bin/env bash

# Sourced by run-m1 after its private session and explicit settings are ready.
# Do not enable xtrace here: command payloads include the enrolled account ID.

m1_cli() {
    local seconds=10 remaining
    if [[ -n ${M1_COMMAND_DEADLINE:-} ]]; then
        remaining=$((M1_COMMAND_DEADLINE - SECONDS))
        ((remaining > 0)) || return 124
        ((remaining >= seconds)) || seconds=$remaining
    fi
    timeout --kill-after=1s "${seconds}s" \
        "$FORGE_ROOT_DIR/target/debug/forge-cli" --socket "$M1_ROOT/api.sock" "$@"
}

m1_uuid7() {
    local timestamp random
    timestamp=$(date +%s%3N) || return 1
    [[ $timestamp =~ ^[0-9]{13}$ ]] || return 1
    printf -v timestamp '%012x' "$timestamp"
    random=$(uuidgen --random) || return 1
    random=${random//-/}
    [[ $random =~ ^[0-9a-f]{32}$ ]] || return 1
    printf '%s-%s-7%s-%s-%s\n' "${timestamp:0:8}" "${timestamp:8:4}" \
        "${random:13:3}" "${random:16:4}" "${random:20:12}"
}

m1_command_error() {
    printf 'run-m1: local command could not be confirmed; inspect the private session.\n' >&2
    return 1
}

m1_read() {
    local response
    response=$(m1_cli get "$1" 2>/dev/null) || { m1_command_error; return 1; }
    jq -e 'type == "object"' >/dev/null 2>&1 <<<"$response" || {
        m1_command_error; return 1;
    }
    printf '%s\n' "$response"
}

m1_command() {
    local name=$1 payload=$2 key revision response attempt
    local conflict='^forge: local Core API returned HTTP 409: domain transition rejected: stale project revision: expected ([0-9]+), current ([0-9]+)$'
    key=$(m1_uuid7) || { m1_command_error; return 1; }
    for attempt in 1 2; do
        revision=0
        if [[ $name != create_project ]]; then
            response=$(m1_read "/v1/projects/$M1_PROJECT_ID") || return 1
            revision=$(jq -er '.revision | select(type == "number" and . >= 1 and floor == .)' \
                <<<"$response" 2>/dev/null) || { m1_command_error; return 1; }
        fi
        if response=$(m1_cli command "$name" --project-id "$M1_PROJECT_ID" \
            --expected-revision "$revision" --idempotency-key "$key" --payload "$payload" 2>&1); then
            jq -e '.status == "applied" or .status == "replayed"' \
                >/dev/null 2>&1 <<<"$response" || { m1_command_error; return 1; }
            printf '%s\n' "$response"
            return 0
        fi
        # The CLI currently omits error codes. Only this exact, known pre-commit
        # rejection permits a revised request with the same retry identity.
        if [[ $name != create_project && $attempt == 1 && $response =~ $conflict \
            && ${BASH_REMATCH[1]} == "$revision" ]]; then
            continue
        fi
        m1_command_error
        return 1
    done
}

m1_resource_id() {
    jq -er --arg kind "$1" '.resource | select(.kind == $kind) | .id |
        select(type == "string" and test("^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))' \
        2>/dev/null
}

m1_record_session() {
    local source=/dev/null temporary
    [[ ! -e $M1_ROOT/session.json ]] || source=$M1_ROOT/session.json
    temporary=$(mktemp "$M1_ROOT/session.XXXXXX") || return 1
    if jq -s --arg project "${M1_PROJECT_ID:-}" --arg task "${M1_TASK_ID:-}" \
        --arg employee "${M1_EMPLOYEE_ID:-}" --arg pipeline "${M1_PIPELINE_VERSION_ID:-}" \
        --arg database "${M1_DATABASE:-}" --arg host "${M1_HOST:-}" \
        --arg image "${M1_IMAGE:-}" --arg model "${M1_MODEL:-}" \
        '(.[0] // {}) + {project_id:$project, task_id:$task, employee_id:$employee,
            pipeline_version_id:$pipeline, database:$database, host_id:$host,
            runtime_image:$image, model:$model}' "$source" >"$temporary" 2>/dev/null; then
        chmod 600 "$temporary" && mv -- "$temporary" "$M1_ROOT/session.json"
    else
        rm -f -- "$temporary"
        m1_command_error
    fi
}

m1_seed() {
    local response payload pipeline_id secret_id binding_id profile_id task_revision
    M1_PROJECT_ID=$(m1_uuid7) || return 1
    M1_TASK_ID= M1_EMPLOYEE_ID= M1_PIPELINE_VERSION_ID=
    m1_record_session || return 1
    m1_command create_project '{"name":"M1 local Codex task"}' >/dev/null || return 1
    response=$(m1_read "/v1/projects/$M1_PROJECT_ID") || return 1
    jq -e '.execution_gate == "stopped"' >/dev/null <<<"$response" || {
        m1_command_error; return 1;
    }
    response=$(m1_command create_pipeline '{
        "name":"M1 evidenced delivery", "task_kinds":["delivery"], "entry_stage_id":"work",
        "stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["completed"]}],
        "transitions":[{"from_stage_id":"work","outcome":"completed","target":{"kind":"done"},
            "artifact_requirements":[{"kind":"stage_evidence","minimum_count":1,"scope":"current_stage"}]}]
    }') || return 1
    pipeline_id=$(m1_resource_id pipeline <<<"$response") || { m1_command_error; return 1; }
    response=$(m1_read "/v1/projects/$M1_PROJECT_ID/pipelines") || return 1
    M1_PIPELINE_VERSION_ID=$(jq -er --arg id "$pipeline_id" \
        '.items[] | select(.pipeline_id == $id) | .id' <<<"$response" 2>/dev/null) || return 1
    response=$(m1_command create_employee '{"name":"M1 Codex executor","role":"executor","stage_eligibility":{"mode":"any"}}') || return 1
    M1_EMPLOYEE_ID=$(m1_resource_id employee <<<"$response") || { m1_command_error; return 1; }
    m1_record_session || return 1
    secret_id=$(m1_uuid7) && binding_id=$(m1_uuid7) && profile_id=$(m1_uuid7) || return 1
    payload=$(jq -cn --arg secret "$secret_id" --arg binding "$binding_id" --arg source "$M1_AUTH" \
        '{secret_id:$secret,binding_id:$binding,kind:"codex_chatgpt",source_file:$source}') || return 1
    m1_command enroll_credential "$payload" >/dev/null || return 1
    # jq reads only the chosen snapshot; only its account identity enters the
    # profile. Raw token fields never become shell variables or command output.
    payload=$(jq -ce --arg project "$M1_PROJECT_ID" --arg employee "$M1_EMPLOYEE_ID" \
        --arg secret "$secret_id" --arg binding "$binding_id" --arg profile "$profile_id" \
        --arg model "$M1_MODEL" --arg image "$M1_IMAGE" '
        .tokens.account_id | select(type == "string" and length > 0 and (test("[[:cntrl:]]") | not)) |
        {employee_id:$employee, binding:{
            execution_profile:{id:$profile,revision:1,project_id:$project,
                adapter_id:"codex_cli",adapter_version:"0.153.2",provider_id:"openai",model:$model,
                credential_binding:{id:$binding,project_id:$project,secret_id:$secret,account_id:.,
                    allowed_delivery_modes:["isolated_runtime_secret"]},
                credential_delivery:"isolated_runtime_secret",
                capability_profile:{adapter_id:"codex_cli",adapter_version:"0.153.2",transport_engine:"cli_wrapper",
                    capabilities:["structured_events","model_selection","usage_reporting","controlled_stop","native_mcp"],
                    credential_exposed_to_run:true}},
            image:$image,surface:{mode:"filesystem_sandbox"},access:"read_write",
            limits:{cpu_millis:2000,memory_bytes:4294967296,pids:256,wall_seconds:600,stop_grace_seconds:5},
            system_prompt:"Work only inside your assigned work surface. Never inspect credentials, runtime homes, or host paths. Use the supplied Forge MCP tools for task evidence and outcomes. No host repository is attached.",
            employee_prompt:"Complete the assigned Task and verify the result. Then call forge_submit_artifact with artifact_kind stage_evidence, a concise title, a structured body describing the result and checks, and a fresh UUIDv7 message_id. Then call forge_submit_outcome with stage_id work, outcome completed, artifact_submission_message_ids containing the artifact message_id, and a different fresh UUIDv7 message_id. Reuse a message_id only for an identical retry. If blocked, call forge_request_human with a concise question and a fresh UUIDv7 message_id. Never claim completion without submitting evidence and outcome."
        }}' "$M1_AUTH" 2>/dev/null) || { m1_command_error; return 1; }
    m1_command configure_employee_runtime "$payload" >/dev/null || return 1
    unset payload
    payload=$(jq -cn --arg pipeline "$M1_PIPELINE_VERSION_ID" --arg task "$M1_TASK_TEXT" \
        '{title:"M1 local task",description:$task,definition_of_done:"Requested work is verified and stage_evidence is accepted with the completed outcome.",
          kind:"delivery",pipeline_version_id:$pipeline,priority:"normal"}') || return 1
    response=$(m1_command create_task "$payload") || return 1
    M1_TASK_ID=$(m1_resource_id task <<<"$response") || { m1_command_error; return 1; }
    m1_record_session || return 1
    response=$(m1_read "/v1/projects/$M1_PROJECT_ID/tasks/$M1_TASK_ID") || return 1
    task_revision=$(jq -er '.revision | select(type == "number" and . >= 1 and floor == .)' \
        <<<"$response" 2>/dev/null) || return 1
    payload=$(jq -cn --arg id "$M1_TASK_ID" --argjson revision "$task_revision" \
        '{task_id:$id,expected_task_revision:$revision}') || return 1
    m1_command approve_task "$payload" >/dev/null
}

m1_start_project() {
    m1_command start_project_execution '{"reason":"Operator requested run-m1 execution"}' >/dev/null
}

m1_stop_project() {
    local M1_COMMAND_DEADLINE=$((SECONDS + 12))
    m1_command stop_project_execution '{"reason":"run-m1 session cleanup"}' >/dev/null
}

m1_show_result() {
    local task runs response run_id
    task=$(m1_read "/v1/projects/$M1_PROJECT_ID/tasks/$M1_TASK_ID") || return 1
    runs=$(m1_read "/v1/projects/$M1_PROJECT_ID/runs") || return 1
    # Keep the public task/artifact projection readable without dumping arbitrary
    # provider prose into the terminal. It contains no raw runtime logs.
    printf '%s\n' "$task" >"$M1_ROOT/result.json" || return 1
    chmod 600 "$M1_ROOT/result.json" || return 1
    printf 'Результат и артефакты: %s/result.json\nРабочие файлы: %s/exec/surfaces/\n' \
        "$M1_ROOT" "$M1_ROOT" >&2
    # Artifact/provider prose is intentionally absent from this terminal view:
    # bounded IDs and typed status fields cannot print raw logs or auth material.
    jq --arg project "$M1_PROJECT_ID" '
        def id: if type == "string" and test("^[0-9a-f-]{36}$") then . else null end;
        def lifecycle: if . == "draft" or . == "ready" or . == "in_progress" or . == "waiting" or
            . == "done" or . == "cancelled" then . else "unknown" end;
        {project_id:$project,task_id:(.id|id),lifecycle:(.lifecycle|lifecycle),
         artifact_count:(.artifacts|length),artifacts:[.artifacts[:10][] |
            {id:(.id|id),kind:(if .kind == "stage_evidence" then "stage_evidence" else "other" end)}],
         wait_condition_count:(.wait_conditions|length)}' <<<"$task" 2>/dev/null || return 1
    while IFS= read -r run_id; do
        [[ -n $run_id ]] || continue
        response=$(m1_read "/v1/projects/$M1_PROJECT_ID/runs/$run_id") || return 1
        jq --arg run "$run_id" '
            def state: if . == "running" or . == "stopped" or . == "provision_requested" or
                . == "stop_requested" or . == "force_stop_requested" or . == "provisioning" or
                . == "stopping" or . == "lost" or . == "failed" or . == "unknown" then . else "other" end;
            {run_id:$run,desired_state:(.desired_state|state),observed_state:(.observed_state|state),
             runtime_report_available:(.diagnostics.runtime_report != null),
             evidence_object_count:(.diagnostics.evidence|length),
             incident_count:(.diagnostics.incidents|length),
             incomplete_stream_count:([.diagnostics.streams[]? | select(.incomplete == true)]|length)}
        ' <<<"$response" 2>/dev/null || return 1
    done < <(jq -r --arg task "$M1_TASK_ID" '[.items[] | select(.task_id == $task)] | .[-10:][] | .id |
        select(type == "string" and test("^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))' \
        <<<"$runs" 2>/dev/null)
}
