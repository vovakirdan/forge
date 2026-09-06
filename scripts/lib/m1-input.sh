#!/usr/bin/env bash
# Read only an explicitly selected auth descriptor; never import host CLI config.

m1_reset_launcher_state() {
    M1_APPROVED=false
    M1_ROOT=''
    M1_EXEC=''
    M1_HOST=''
    M1_DATABASE=''
    M1_PROJECT_ID=''
    M1_AUTH_SOURCE=''
    M1_AUTH=''
    M1_MODEL=''
    M1_TASK_TEXT=''
    M1_CORE_PID=''
    M1_CORE_TOKEN=''
    M1_SUPERVISOR_PID=''
    M1_SUPERVISOR_TOKEN=''
    M1_CLEANUP_DONE=0
    M1_CLEANUP_STATUS=0
    # Never close a descriptor or inherit a deadline we did not create.
    unset M1_AUTH_FD M1_COMMAND_DEADLINE
}

m1_model_hint() {
    local config="$1"
    [[ -f "$config" && ! -L "$config" ]] || return 0
    # A hint, not a TOML loader: only a simple top-level model is accepted.
    # Profiles, providers, hooks and other settings are deliberately not imported.
    awk '
        /^[[:space:]]*\[/ { exit }
        /^[[:space:]]*model[[:space:]]*=[[:space:]]*"[a-zA-Z0-9._:\/-]+"/ {
            if (match($0, /"[a-zA-Z0-9._:\/-]+"/))
                print substr($0, RSTART + 1, RLENGTH - 2)
            exit
        }
    ' "$config"
}

m1_close_auth() {
    if [[ -n ${M1_AUTH_FD:-} ]]; then
        exec {M1_AUTH_FD}<&-
        unset M1_AUTH_FD
    fi
}

m1_open_auth() {
    local source="$1" metadata owner mode links size kind
    [[ "$source" = /* && -f "$source" && ! -L "$source" ]] || {
        printf 'Нужен абсолютный путь к обычному auth.json, не symlink.\n' >&2
        return 1
    }
    exec {M1_AUTH_FD}<"$source" || return
    # Validate the opened inode, not a pathname that can be replaced before copy.
    metadata=$(LC_ALL=C stat -Lc '%u %a %h %s %F' "/proc/$BASHPID/fd/$M1_AUTH_FD") || {
        m1_close_auth; return 1
    }
    read -r owner mode links size kind <<<"$metadata"
    if [[ "$owner" != "$(id -u)" || "$mode" != 600 || "$links" != 1 \
        || "$kind" != 'regular file' || "$size" -gt 1048576 ]]; then
        printf 'Auth должен принадлежать тебе, иметь mode 0600, одну ссылку и размер до 1 MiB.\n' >&2
        m1_close_auth
        return 1
    fi
    # Opening /proc/.../fd reads the selected inode without moving the held fd.
    if ! jq -e '
        (.auth_mode == null or .auth_mode == "chatgpt") and
        .OPENAI_API_KEY == null and
        (.last_refresh | type == "string" and length > 0) and
        ([.tokens.access_token, .tokens.refresh_token, .tokens.id_token,
          .tokens.account_id] | all(type == "string" and length > 0))
    ' "/proc/$BASHPID/fd/$M1_AUTH_FD" >/dev/null 2>&1; then
        printf 'Файл не содержит поддерживаемую ChatGPT-авторизацию Codex. Выполни codex login или выбери другой --auth-file.\n' >&2
        m1_close_auth
        return 1
    fi
}

m1_copy_auth() {
    [[ -n ${M1_AUTH_FD:-} && ! -e "$M1_AUTH" ]] || return 1
    # The private session is fresh; the original auth is never opened for writing.
    install -m 600 "/proc/$BASHPID/fd/$M1_AUTH_FD" "$M1_AUTH" || return
    m1_close_auth
}

m1_remove_import_copy() {
    # Only this launcher's fresh input copy; Core has already encrypted it and
    # issued its own immutable credential version. Never remove the source auth.
    [[ "$M1_AUTH" == "$M1_ROOT/auth.json" && -f "$M1_AUTH" && ! -L "$M1_AUTH" \
        && $(stat -c '%u:%a:%h' "$M1_AUTH") == "$(id -u):600:1" ]] || return 1
    rm -- "$M1_AUTH"
}

m1_prepare_inputs() {
    local configured_home="${CODEX_HOME:-$HOME/.codex}" hint answer
    if [[ -z ${M1_AUTH_SOURCE:-} ]]; then
        [[ -t 0 ]] || { printf 'Noninteractive: specify --auth-file.\n' >&2; return 1; }
        M1_AUTH_SOURCE="$configured_home/auth.json"
        if [[ ! -f "$M1_AUTH_SOURCE" ]]; then
            printf 'Текущий auth.json не найден. Выполни codex login, затем повтори just run-m1; либо передай --auth-file PATH.\n' >&2
            return 1
        fi
    fi
    if [[ -z ${M1_MODEL:-} ]]; then
        [[ -t 0 ]] || { printf 'Noninteractive: specify --model.\n' >&2; return 1; }
        hint=$(m1_model_hint "$configured_home/config.toml")
        read -r -p "Модель Codex${hint:+ [$hint]}: " M1_MODEL || return
        M1_MODEL="${M1_MODEL:-$hint}"
    fi
    [[ "$M1_MODEL" =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/-]*$ ]] || {
        printf 'Укажи явное имя модели; автоматической подмены нет.\n' >&2; return 1
    }
    if [[ -z ${M1_TASK_TEXT:-} ]]; then
        [[ -t 0 ]] || { printf 'Noninteractive: specify --task.\n' >&2; return 1; }
        read -r -p 'Что выполнить в пустом рабочем каталоге? ' M1_TASK_TEXT || return
    fi
    [[ "$M1_TASK_TEXT" =~ [^[:space:]] && ${#M1_TASK_TEXT} -le 20000 ]] || {
        printf 'Нужен непустой текст задачи до 20000 символов.\n' >&2; return 1
    }
    printf 'Codex: %s\nAuth source: %s\nОдин изолированный Run; до 10 минут при работающем наблюдателе. Используется лимит подписки.\n' \
        "$M1_MODEL" "$M1_AUTH_SOURCE"
    if [[ ${M1_APPROVED:-false} != true ]]; then
        [[ -t 0 ]] || { printf 'Noninteractive launch requires explicit --yes.\n' >&2; return 1; }
        read -r -p 'Использовать эту авторизацию и запустить задачу? [y/N] ' answer || return
        [[ "$answer" == y || "$answer" == Y || "$answer" == да ]] || return 1
    fi
    # No credential bytes are read before the operator sees and approves the source.
    m1_open_auth "$M1_AUTH_SOURCE"
}
