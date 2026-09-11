#!/usr/bin/env bash
# Private JSON state is data, never sourced as shell code.

m2_error() { printf 'forge M2: %s\n' "$*" >&2; return 1; }
m2_usage() {
    printf '%s\n' \
        'Usage: bash /path/to/forge/scripts/run-m2.sh init [--project-dir EMPTY_GIT_DIRECTORY]' \
        '       ./forge-m2 run [--yes] [--lane LANE] [--model MODEL] [--auth-file FILE]' \
        '       ./forge-m2 status|stop|evidence' \
        'Lanes: codex_cli, claude_cli, openrouter_api, openai_api.' \
        'run asks for explicit provider/model/auth approval. API lanes need a real local LiteLLM.' \
        'No login, source-auth writeback, automatic paid retry, or application code generation by this script.'
}

m2_private_file() {
    [[ $1 == /* && $1 != *[[:cntrl:]]* && -f $1 && ! -L $1 && $(stat -c '%u:%a:%h' "$1") == "$(id -u):600:1" ]] \
        || { m2_error 'Нужен принадлежащий тебе обычный файл 0600 с одной ссылкой, не symlink.'; return 1; }
}

m2_private_directory() {
    [[ $1 == /* && -d $1 && ! -L $1 && $(stat -c '%u:%a' "$1") == "$(id -u):700" ]] \
        || { m2_error 'Нужен принадлежащий тебе приватный каталог 0700, не symlink.'; return 1; }
}

m2_json_write() {
    local destination=$1 value=$2 temporary
    [[ ! -L $destination && ! -e $destination || -f $destination && ! -L $destination ]] || return 1
    value=$(jq -se 'if length==1 and (.[0]|type=="object" or type=="array") then .[0]
        else error("invalid snapshot") end' <<<"$value" 2>/dev/null) \
        || { m2_error 'JSON snapshot не сохранён: требуется один объект или массив.'; return 1; }
    temporary=$(mktemp "${destination}.XXXXXX") || return 1
    printf '%s\n' "$value" >"$temporary" || return 1
    chmod 600 "$temporary" && mv -- "$temporary" "$destination"
}

m2_git() {
    # Only controller-owned Git administration; never execute project hooks/filters.
    env -i PATH=/usr/bin:/bin LC_ALL=C GIT_CONFIG_NOSYSTEM=1 \
        GIT_CONFIG_SYSTEM=/dev/null GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
        git -c core.hooksPath=/dev/null -c core.fsmonitor=false -c commit.gpgSign=false "$@"
}

m2_bind_session() {
    M1_ROOT=$M2_ROOT M1_EXEC="$M2_ROOT/exec" M1_PROJECT_ID=${M2_PROJECT_ID:-}
    M1_HOST=${M2_HOST:-} M1_CLEANUP_DONE=0 M1_CLEANUP_STATUS=0
    M1_CORE_PID='' M1_CORE_TOKEN='' M1_SUPERVISOR_PID='' M1_SUPERVISOR_TOKEN=''
}

m2_command() { m1_command "$@"; }
m2_read() { m1_read "$@"; }
m2_resource() { m1_resource_id "$1"; }

m2_pages() {
    local path=$1 response cursor='' result='[]' count=0 separator='?' seen='|'
    [[ $path != *\?* ]] || separator='&'
    while :; do
        response=$(m2_read "${path}${separator}limit=100${cursor:+&after=$cursor}") || return 1
        response=$(jq -cse 'if length==1 and (.[0]|type=="object" and (.items|type=="array") and all(.items[];type=="object")
            and (.next_cursor==null or (.next_cursor|type=="string"))) then .[0] else error("invalid page") end' \
            <<<"$response" 2>/dev/null) || { m2_error 'Некорректная страница API.'; return 1; }
        result=$(printf '%s\n' "$result" "$response" | jq -ces '.[0] + .[1].items') || return 1
        cursor=$(jq -er '.next_cursor // ""' <<<"$response") || return 1
        [[ -n $cursor ]] || break
        [[ $cursor =~ ^[0-9a-f-]+$ && $seen != *"|$cursor|"* && $((++count)) -lt 20 ]] \
            || { m2_error 'Неожиданный/repeated cursor или слишком много страниц.'; return 1; }
        seen+="$cursor|"
    done
    jq -c '{items:.}' <<<"$result"
}

m2_record() {
    local existing='{}' record
    [[ ! -f $M2_ROOT/session.json ]] || existing=$(jq -ce . "$M2_ROOT/session.json") || return 1
    record=$(printf '%s\n' "$existing" "${1:-\{\}}" | jq -cs \
        --arg root "$M2_ROOT" --arg project "${M2_PROJECT_ID:-}" --arg host "${M2_HOST:-}" \
        --arg database "${M2_DATABASE:-}" --arg source "$M2_TARGET" --arg image "${M2_IMAGE:-}" \
        --arg lane "${M2_LANE:-}" --arg model "${M2_MODEL:-}" \
        --argjson pid "$M1_LAUNCHER_PID" --arg token "$(m1_process_token "$M1_LAUNCHER_PID")" \
        'if length==2 and all(.[];type=="object") then
            .[0] + {root:$root,project_id:$project,host_id:$host,database:$database,target:$source,
                image:$image,lane:$lane,model:$model,launcher_pid:$pid,launcher_token:$token} + .[1]
            else error("invalid session patch") end' 2>/dev/null) \
        || { m2_error 'Некорректный JSON session patch.'; return 1; }
    m2_json_write "$M2_ROOT/session.json" "$record"
}

m2_load_session() {
    m2_private_file "$M2_PRIVATE/current.json" || return 1
    M2_ROOT=$(jq -er '.root | select(type == "string")' "$M2_PRIVATE/current.json") || return 1
    [[ $M2_ROOT =~ ^/tmp/fm2\.[a-zA-Z0-9]{8}$ ]] || { m2_error 'Неожиданный путь сессии.'; return 1; }
    m2_private_directory "$M2_ROOT" && m2_private_file "$M2_ROOT/session.json" || return 1
    M2_PROJECT_ID=$(jq -er '.project_id' "$M2_ROOT/session.json") || return 1
    M2_HOST=$(jq -er '.host_id' "$M2_ROOT/session.json") || return 1
    [[ $M2_PROJECT_ID =~ ^[0-9a-f-]{36}$ && $M2_HOST =~ ^m2-manual-[0-9a-f-]{36}$ ]] || return 1
    [[ $(jq -er '.target' "$M2_ROOT/session.json") == "$M2_TARGET" ]] || return 1
    m2_bind_session
}
