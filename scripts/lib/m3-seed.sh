#!/usr/bin/env bash

m3_profile_template() {
    "$FORGE_ROOT_DIR/target/debug/forge-cli" profile-template --payload "$1" 2>"$M1_ROOT/profile-error.log"
}

m3_publish() {
    local title=$1 markdown=$2 page payload
    page=$(m1_uuid7)
    payload=$(jq -cn --arg id "$page" --arg title "$title" --arg markdown "$markdown" \
        '{page_id:$id,expected_page_revision:0,kind:"policy",title:$title,markdown:$markdown,source_refs:[]}')
    m1_command author_knowledge_page "$payload" >/dev/null
    m1_command publish_knowledge_page "$(jq -cn --arg id "$page" '{page_id:$id,expected_page_revision:1}')" >/dev/null
    printf '%s\n' "$page"
}

m3_seed_employee() {
    local secret binding template payload response pipeline account
    secret=$(m1_uuid7) binding=$(m1_uuid7)
    account=$(jq -er '.tokens.account_id' "$M1_AUTH")
    payload=$(jq -cn --arg secret "$secret" --arg binding "$binding" --arg source "$M1_AUTH" \
        '{secret_id:$secret,binding_id:$binding,kind:"codex_chatgpt",source_file:$source}')
    m1_command enroll_credential "$payload" >/dev/null
    m1_remove_import_copy
    # Only the CLI owns adapter-version/capability selection. A template has no side effects.
    template=$(jq -cn --arg project "$M1_PROJECT_ID" --arg employee "$(m1_uuid7)" --arg secret "$secret" --arg binding "$binding" \
        --arg account "$account" --arg model "$M1_MODEL" --arg image "$M1_IMAGE" \
        '{lane:"codex_cli",project_id:$project,employee_id:$employee,secret_id:$secret,binding_id:$binding,account_id:$account,
          model:$model,image:$image,access:"read_write",live_input:false,
          limits:{cpu_millis:2000,memory_bytes:4294967296,pids:256,wall_seconds:600,stop_grace_seconds:5},
          budget:{max_output_bytes:1048576,requests_per_minute:20,tokens_per_minute:20000,max_spend_microusd:null},
          system_prompt:"Work only within the assigned isolated environment and scoped Forge tools. Never inspect credentials, host paths or another runtime. Follow the pinned assignment and canonical knowledge; an index is not authority.",
          employee_prompt:"For a TaskStage assignment, complete the requested work, check it using the available shell tools, then submit a stage_evidence artifact with a concise report and a fresh UUIDv7 message_id, followed by forge_submit_outcome for stage work and outcome completed with artifact_submission_message_ids. Never invent completion. A SystemJob has separate instructions and its own submit_result tool; it has no Task ownership."}')
    m3_profile_template "$template" >"$M1_ROOT/profile.json"
    jq '.binding | .surface={mode:"none"} | .access="read_only" | .limits.wall_seconds=300' \
        "$M1_ROOT/profile.json" >"$M1_ROOT/system-job-binding.json"
    payload=$(jq -cn --slurpfile binding "$M1_ROOT/system-job-binding.json" \
        '{expected_settings_revision:0,binding:$binding[0],policy:{enabled:true,max_concurrent:1,max_attempts_per_job:1,max_attempts_per_day:4,max_input_bytes:65536,max_result_bytes:16384,wall_seconds:300,coalesce_seconds:30}}')
    m1_command configure_system_jobs "$payload" >/dev/null
    # Create after configuration; never reuse M1/M2's explicit onboarding bypass.
    response=$(m1_command create_employee '{"name":"M3 Codex employee","role":"executor","stage_eligibility":{"mode":"any"}}')
    M1_EMPLOYEE_ID=$(m1_resource_id employee <<<"$response")
    payload=$(jq -c --arg employee "$M1_EMPLOYEE_ID" '.employee_id=$employee | .binding.surface={mode:"filesystem_sandbox"}' "$M1_ROOT/profile.json")
    m1_command configure_employee_runtime "$payload" >/dev/null
    m1_read "/v1/projects/$M1_PROJECT_ID/employees/$M1_EMPLOYEE_ID/onboarding" >"$M1_ROOT/evidence/onboarding-before.json"
    jq -e '.state=="pending"' "$M1_ROOT/evidence/onboarding-before.json" >/dev/null
    response=$(m1_command create_pipeline '{"name":"M3 evidenced work","task_kinds":["delivery"],"entry_stage_id":"work",
        "stages":[{"id":"work","name":"Work","executor_kind":"employee","outcomes":["completed"]}],
        "transitions":[{"from_stage_id":"work","outcome":"completed","target":{"kind":"done"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1,"scope":"current_stage"}]}]}')
    pipeline=$(m1_resource_id pipeline <<<"$response")
    M1_PIPELINE_VERSION_ID=$(m1_read "/v1/projects/$M1_PROJECT_ID/pipelines" | jq -er --arg id "$pipeline" '.items[]|select(.pipeline_id==$id)|.id')
    m1_record_session
}

m3_seed_task() {
    local payload response revision
    payload=$(jq -cn --arg pipeline "$M1_PIPELINE_VERSION_ID" \
        '{title:"M3 normalize lines",description:"Read the canonical knowledge and your available personal memory through Forge tools. In your isolated work surface create normalize-lines.sh: an executable POSIX shell script reading stdin, removing exact duplicates and sorting under LC_ALL=C. Empty input must produce empty output. Use only shell and sort, no installation. Check it, and attach stage_evidence describing actual work and observed checks; mention the violet compass policy if delivered. Then submit completed for work. Never fabricate memory reads or test results.",definition_of_done:"The script and accepted evidence exist; observed checks are described.",kind:"delivery",pipeline_version_id:$pipeline,priority:"normal"}')
    response=$(m1_command create_task "$payload")
    M1_TASK_ID=$(m1_resource_id task <<<"$response")
    revision=$(m1_read "/v1/projects/$M1_PROJECT_ID/tasks/$M1_TASK_ID" | jq -er '.revision')
    m1_command approve_task "$(jq -cn --arg task "$M1_TASK_ID" --argjson revision "$revision" '{task_id:$task,expected_task_revision:$revision}')" >/dev/null
    m1_record_session
}
