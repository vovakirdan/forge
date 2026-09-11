#!/usr/bin/env bash

m2_profile_template() {
    "$FORGE_ROOT_DIR/target/debug/forge-cli" profile-template --payload "$1" 2>"$M2_ROOT/profile-error.log"
}

m2_seed_employee() {
    local role=$1 capacity=$2 response employee payload account=null
    payload=$(jq -cn --arg role "$role" --arg version "$M2_PIPELINE" \
        '{name:("M2 " + $role),role:$role,stage_eligibility:{mode:"only",stages:[{pipeline_version_id:$version,stage_id:$role}]}}')
    response=$(m2_command create_employee "$payload") || return 1
    employee=$(m2_resource employee <<<"$response") || return 1
    if [[ $role == work ]]; then
        m2_command amend_employee "$(jq -cn --arg id "$employee" --argjson capacity "$capacity" \
            '{employee_id:$id,expected_employee_revision:1,patch:{max_concurrent_runs:$capacity}}')" >/dev/null || return 1
    else
        m2_command disable_employee "$(jq -cn --arg id "$employee" '{employee_id:$id,expected_employee_revision:1}')" >/dev/null || return 1
    fi
    [[ $M2_LANE == *_api ]] || account=$(jq -cn --arg value "$M2_ACCOUNT" '$value')
    payload=$(jq -cn --arg lane "$M2_LANE" --arg project "$M2_PROJECT_ID" --arg employee "$employee" \
        --arg binding "$M2_BINDING" --arg secret "$M2_SECRET" --argjson account "$account" \
        --arg model "$M2_MODEL" --arg image "$M2_IMAGE" --argjson wall "$M2_RUN_WALL" \
        --argjson spend "${M2_SPEND:-null}" --arg role "$role" \
        '{lane:$lane,project_id:$project,employee_id:$employee,binding_id:$binding,secret_id:$secret,
          account_id:$account,model:$model,image:$image,live_input:true,
          access:(if $role=="work" then "read_write" else "read_only" end),
          limits:{cpu_millis:2000,memory_bytes:4294967296,pids:256,wall_seconds:$wall,stop_grace_seconds:5},
          budget:{max_output_bytes:8388608,requests_per_minute:20,tokens_per_minute:20000,max_spend_microusd:$spend},
          system_prompt:"Work only inside your assigned Forge environment, Task surface and explicitly delivered read-only inputs. Never inspect credentials, host paths, other worker files or environment secrets. Use Forge MCP for all Task/Inbox actions. Never run background loops or sleep while awaiting native input. Context and stage instructions are authoritative. Human safety questions must be escalated, not guessed.",
          employee_prompt:"For TaskStage: follow the pinned stage instructions; read required instructions with forge_list_instructions, answer with forge_reply_instruction using target_message_id and fresh UUIDv7 message_id. READY/GO applies only to the initial work visit. After GO, or on any later rework/review/QA visit, complete actual work. Submit forge_submit_artifact with the stage-required artifact_kind, concise title, structured report body and fresh UUIDv7 message_id; then forge_submit_outcome with current stage_id, allowed outcome, exact candidate_commit and artifact_submission_message_ids. Use a new UUIDv7 per distinct call and reuse it only for identical retry. A Git proposal receipt is not acceptance: stop working after submission. For a taskless Communication assignment: read its source Inbox question, answer using the scoped reply tool and target_message_id, then call forge_complete_communication with empty arguments; do not wait for GO, edit Task files, or submit Task artifacts/outcomes. For blocked work use the scoped escalation tool; never fabricate a verdict."}')
    # CLI derives pinned provider versions/capabilities; do not maintain another list.
    payload=$(m2_profile_template "$payload") || return 1
    m2_command configure_employee_runtime "$payload" >/dev/null || return 1
    printf '%s\n' "$employee"
}

m2_seed_task() {
    local title=$1 description=$2 response task revision payload
    payload=$(jq -cn --arg title "$title" --arg description "$description" --arg pipeline "$M2_PIPELINE" \
        '{title:$title,description:$description,definition_of_done:"Own-file implementation is committed, independently reviewed, checked by report-only QA and integrated. After stale-base rework the combined candidate preserves both independently developed files and repeats fresh review/QA.",kind:"delivery",pipeline_version_id:$pipeline,priority:"normal"}')
    response=$(m2_command create_task "$payload") || return 1
    task=$(m2_resource task <<<"$response") || return 1
    response=$(m2_read "/v1/projects/$M2_PROJECT_ID/tasks/$task") || return 1
    revision=$(jq -er '.revision' <<<"$response") || return 1
    payload=$(jq -cn --arg task "$task" --arg repository "$M2_REPOSITORY" --argjson revision "$revision" \
        '{task_id:$task,expected_task_revision:$revision,repository_id:$repository,initial_base:{kind:"unborn",object_format:"sha1"}}')
    m2_command bind_task_git_repository "$payload" >/dev/null || return 1
    response=$(m2_read "/v1/projects/$M2_PROJECT_ID/tasks/$task") || return 1
    revision=$(jq -er '.revision' <<<"$response") || return 1
    m2_command approve_task "$(jq -cn --arg task "$task" --argjson revision "$revision" '{task_id:$task,expected_task_revision:$revision}')" >/dev/null || return 1
    printf '%s\n' "$task"
}

m2_open_thread() {
    local employee=$1 task=${2:-} payload
    payload=$(jq -cn --arg employee "$employee" --arg task "$task" \
        '{employee_id:$employee} + (if $task=="" then {} else {task_id:$task} end)')
    m2_command open_employee_thread "$payload" | m2_resource employee_thread
}

m2_send() {
    local thread=$1 revision=$2 target=$3 body=$4 kind=${5:-instruction} payload
    payload=$(jq -cn --arg thread "$thread" --argjson revision "$revision" --argjson target "$target" \
        --arg body "$body" --arg kind "$kind" \
        '{thread_id:$thread,expected_thread_revision:$revision,target:$target,kind:$kind,requirement:"answered",body:$body}')
    m2_command send_employee_message "$payload" | m2_resource employee_message
}

m2_seed() {
    local response pipeline_id payload kind initial_target
    m2_command create_project '{"name":"M2 live isolated Git exercise"}' >/dev/null
    response=$(m2_command create_pipeline "$(jq -c . "$SCRIPT_DIR/lib/m2-pipeline.json")")
    pipeline_id=$(m2_resource pipeline <<<"$response")
    response=$(m2_pages "/v1/projects/$M2_PROJECT_ID/pipelines")
    M2_PIPELINE=$(jq -er --arg id "$pipeline_id" '[.items[] | select(.pipeline_id==$id)] | select(length==1) | .[0].id' <<<"$response")
    M2_SECRET=$(m1_uuid7) M2_BINDING=$(m1_uuid7)
    case "$M2_LANE" in codex_cli) kind=codex_chatgpt ;; claude_cli) kind=claude_subscription ;; *) kind=api_key ;; esac
    payload=$(jq -cn --arg secret "$M2_SECRET" --arg binding "$M2_BINDING" --arg kind "$kind" --arg file "$M2_ENROLL_FILE" \
        '{secret_id:$secret,binding_id:$binding,kind:$kind,source_file:$file}')
    m2_command enroll_credential "$payload" >/dev/null
    [[ $M2_LANE != codex_cli ]] || m1_remove_import_copy
    M2_WRITER=$(m2_seed_employee work 3)
    M2_REVIEWER=$(m2_seed_employee review 1)
    M2_QA=$(m2_seed_employee qa 1)
    response=$(m2_command register_project_repository "$(jq -cn --arg source "$M2_TARGET" \
        '{name:"M2 empty isolated target",source:$source,target_ref:"refs/heads/main"}')")
    M2_REPOSITORY=$(m2_resource project_repository <<<"$response")
    M2_TASK_A=$(m2_seed_task 'Create normalize-lines.sh' \
        'This is Task A in an empty Git repository. On the first work visit create ONLY normalize-lines.sh: an executable POSIX-shell command reading text lines from stdin, removing exact duplicate lines and outputting sorted lines under LC_ALL=C. Empty input yields empty output. Use only POSIX shell and sort; no package installation. Choose and run appropriate checks in your own sandbox. Do not add README.md, tests or READY marker files. First answer READY to the initial instruction and wait for addressed GO. On later stale-base rework fetch the delivered new target bundle, explicitly merge its independent history with --allow-unrelated-histories if required, preserve README.md and your script, check and commit the combined result.')
    M2_TASK_B=$(m2_seed_task 'Write README.md' \
        'This is Task B in an empty Git repository. On the first work visit create ONLY README.md documenting the future ./normalize-lines.sh command: reads stdin, removes exact duplicate lines, sorts under LC_ALL=C, empty input, dependencies POSIX shell and sort, useful usage examples. The script is implemented by another Task: do not create it yourself and do not access its workspace. Do not add tests or READY marker files. First answer READY to the initial instruction and wait for addressed GO. On later stale-base rework fetch the delivered new target bundle, explicitly merge its independent history with --allow-unrelated-histories if required, preserve the real script and your README, verify documentation against the combined candidate and commit.')
    M2_THREAD_A=$(m2_open_thread "$M2_WRITER" "$M2_TASK_A")
    M2_THREAD_B=$(m2_open_thread "$M2_WRITER" "$M2_TASK_B")
    initial_target=$(jq -cn --arg task "$M2_TASK_A" --arg pipeline "$M2_PIPELINE" '{kind:"task_execution",context:{task_id:$task,pipeline_version_id:$pipeline,stage_id:"work",stage_visit:1}}')
    M2_READY_A=$(m2_send "$M2_THREAD_A" 1 "$initial_target" 'Reply exactly READY through forge_reply_instruction to this message, then await your own addressed GO before editing anything. Do not sleep, loop or create marker files.')
    initial_target=$(jq -cn --arg task "$M2_TASK_B" --arg pipeline "$M2_PIPELINE" '{kind:"task_execution",context:{task_id:$task,pipeline_version_id:$pipeline,stage_id:"work",stage_visit:1}}')
    M2_READY_B=$(m2_send "$M2_THREAD_B" 1 "$initial_target" 'Reply exactly READY through forge_reply_instruction to this message, then await your own addressed GO before editing anything. Do not sleep, loop or create marker files.')
    m2_record "$(jq -cn --arg writer "$M2_WRITER" --arg reviewer "$M2_REVIEWER" --arg qa "$M2_QA" \
        --arg pipeline "$M2_PIPELINE" --arg a "$M2_TASK_A" --arg b "$M2_TASK_B" \
        --arg ta "$M2_THREAD_A" --arg tb "$M2_THREAD_B" --arg ra "$M2_READY_A" --arg rb "$M2_READY_B" \
        '{writer_id:$writer,reviewer_id:$reviewer,qa_id:$qa,pipeline_version_id:$pipeline,task_ids:[$a,$b],
            task_thread_ids:[$ta,$tb],ready_message_ids:[$ra,$rb],phase:"seeded"}')"
    printf 'Project: %s\nTask A: %s\nTask B: %s\nPipeline: work → review → qa → integrate → done\n' "$M2_PROJECT_ID" "$M2_TASK_A" "$M2_TASK_B"
}
