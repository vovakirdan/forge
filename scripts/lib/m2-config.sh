#!/usr/bin/env bash

m2_init() {
    local entries exclude wrapper
    [[ -d $M2_PROJECT_DIR/.git && ! -L $M2_PROJECT_DIR/.git ]] \
        || { m2_error 'init требует отдельный git init checkout, не worktree и не Forge.'; return 1; }
    [[ $M2_PROJECT_DIR != "$FORGE_ROOT_DIR" && $M2_PROJECT_DIR != "$FORGE_ROOT_DIR/"* ]] \
        || { m2_error 'Используй отдельный пустой каталог вне Forge checkout.'; return 1; }
    [[ ! -e $M2_PRIVATE && ! -L $M2_PRIVATE && ! -e $M2_PROJECT_DIR/forge-m2 ]] \
        || { m2_error 'M2 scaffold уже существует; init ничего не перезаписывает.'; return 1; }
    [[ $(m2_git -C "$M2_PROJECT_DIR" symbolic-ref HEAD) == refs/heads/main ]] || return 1
    if m2_git -C "$M2_PROJECT_DIR" rev-parse --verify HEAD >/dev/null 2>&1; then
        m2_error 'Нужен unborn checkout без коммитов.'; return 1
    fi
    entries=$(find "$M2_PROJECT_DIR" -mindepth 1 -maxdepth 1 ! -name .git -print -quit)
    [[ -z $entries ]] || { m2_error 'Checkout должен быть пустым: приложение напишут Employees.'; return 1; }
    exclude="$M2_PROJECT_DIR/.git/info/exclude"
    [[ ! -L $M2_PROJECT_DIR/.git/info && ! -L $exclude ]] || return 1
    [[ ! -e $M2_PROJECT_DIR/.git/info || -d $M2_PROJECT_DIR/.git/info ]] || return 1
    install -d -m 700 "$M2_PROJECT_DIR/.git/info"
    install -d -m 700 "$M2_PRIVATE"
    m2_git init --quiet --bare --template= --object-format=sha1 --initial-branch=main "$M2_TARGET"
    m2_json_write "$M2_PRIVATE/config.json" '{"schema_version":1,"lane":null,"model":null,
        "credential_file":null,"account_id":null,"proxy_endpoint":null,"proxy_master_file":null,
        "max_spend_microusd":null,"run_wall_seconds":600,"session_wall_seconds":1800,"observed_run_guard":12}'
    printf '\n# Private Forge M2 operator exercise\n/.forge-m2/\n/forge-m2\n' >>"$exclude"
    wrapper="$M2_PROJECT_DIR/forge-m2"
    (set -o noclobber; printf '#!/usr/bin/env bash\nexec bash %q "$@" --project-dir %q\n' \
        "$SCRIPT_DIR/run-m2.sh" "$M2_PROJECT_DIR" >"$wrapper")
    chmod 700 "$wrapper"
    printf 'Подготовлено: %s\nПустой target: %s\nДальше: ./forge-m2 run\nКонфигурация: %s/config.json\n' \
        "$M2_PROJECT_DIR" "$M2_TARGET" "$M2_PRIVATE"
}

m2_prompt() {
    local variable=$1 label=$2 default=${3:-} value
    [[ -t 0 && ${M2_APPROVED:-false} != true ]] \
        || { m2_error "Нужно явно указать $variable в config.json или аргументе."; return 1; }
    read -r -p "$label${default:+ [$default]}: " value || return 1
    printf -v "$variable" '%s' "${value:-$default}"
}

m2_settings() {
    local config answer chosen_home=${CODEX_HOME:-$HOME/.codex}
    m2_private_file "$M2_PRIVATE/config.json" || return 1
    config=$(jq -ce 'select(.schema_version == 1)' "$M2_PRIVATE/config.json") || return 1
    M2_LANE=${M2_LANE:-$(jq -r '.lane // empty' <<<"$config")}
    M2_MODEL=${M2_MODEL:-$(jq -r '.model // empty' <<<"$config")}
    M2_AUTH_SOURCE=${M2_AUTH_SOURCE:-$(jq -r '.credential_file // empty' <<<"$config")}
    M2_ACCOUNT=${M2_ACCOUNT:-$(jq -r '.account_id // empty' <<<"$config")}
    M2_PROXY=${M2_PROXY:-$(jq -r '.proxy_endpoint // empty' <<<"$config")}
    M2_PROXY_MASTER=${M2_PROXY_MASTER:-$(jq -r '.proxy_master_file // empty' <<<"$config")}
    M2_SPEND=${M2_SPEND:-$(jq -r '.max_spend_microusd // empty' <<<"$config")}
    [[ -n $M2_LANE ]] || m2_prompt M2_LANE 'Провайдер: codex_cli / claude_cli / openrouter_api / openai_api' codex_cli
    case "$M2_LANE" in codex_cli|claude_cli|openrouter_api|openai_api) ;; *) m2_error 'Неизвестная lane.'; return 1 ;; esac
    [[ -n $M2_MODEL ]] || m2_prompt M2_MODEL 'Точная модель (автоподмены нет)'
    [[ $M2_MODEL =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/-]*$ ]] || { m2_error 'Некорректное имя модели.'; return 1; }
    [[ -n $M2_AUTH_SOURCE ]] || m2_prompt M2_AUTH_SOURCE 'Абсолютный путь к auth/token/key файлу' \
        "$(if [[ $M2_LANE == codex_cli ]]; then printf '%s/auth.json' "$chosen_home"; fi)"
    m2_private_file "$M2_AUTH_SOURCE" || return 1
    if [[ $M2_LANE == claude_cli ]]; then
        [[ -n $M2_ACCOUNT ]] || m2_prompt M2_ACCOUNT 'Стабильная метка Claude аккаунта'
        [[ $M2_ACCOUNT =~ ^[a-zA-Z0-9][a-zA-Z0-9._:@/-]{0,199}$ ]] || return 1
    elif [[ $M2_LANE == *_api ]]; then
        [[ -n $M2_PROXY ]] || m2_prompt M2_PROXY 'Адрес настоящего локального LiteLLM' 'http://127.0.0.1:4000'
        [[ $M2_PROXY =~ ^http://(127\.0\.0\.1|localhost):([1-9][0-9]{0,4})/?$ && ${BASH_REMATCH[2]} -le 65535 ]] \
            || { m2_error 'LiteLLM должен быть явным loopback HTTP origin без credentials/query.'; return 1; }
        [[ -n $M2_PROXY_MASTER ]] || m2_prompt M2_PROXY_MASTER 'Абсолютный путь к master-key файлу LiteLLM'
        m2_private_file "$M2_PROXY_MASTER" || return 1
        [[ -n $M2_SPEND ]] || m2_prompt M2_SPEND 'Лимит microUSD на каждый Run (250000 = $0.25)' 250000
        [[ $M2_SPEND =~ ^[1-9][0-9]{0,6}$ && $M2_SPEND -le 5000000 ]] \
            || { m2_error 'API cap должен быть от 1 до 5000000 microUSD на Run.'; return 1; }
    fi
    M2_RUN_WALL=$(jq -er '.run_wall_seconds | select(type=="number" and floor==. and .>=60 and .<=600)' <<<"$config") || return 1
    M2_SESSION_WALL=$(jq -er '.session_wall_seconds | select(type=="number" and floor==. and .>=60 and .<=3600)' <<<"$config") || return 1
    M2_RUN_GUARD=$(jq -er '.observed_run_guard | select(type=="number" and floor==. and .>=10 and .<=20)' <<<"$config") || return 1
    printf 'Lane: %s; модель: %s\nAuth: %s\nGit target: %s\n' "$M2_LANE" "$M2_MODEL" "$M2_AUTH_SOURCE" "$M2_TARGET"
    printf 'Обычно 10 provider Runs; до 3 одновременно. Run: %ss, сессия: %ss при живом launcher.\n' "$M2_RUN_WALL" "$M2_SESSION_WALL"
    printf 'Остановка при наблюдении %s Runs; возможен дополнительный Run между опросами.\n' "$M2_RUN_GUARD"
    if [[ $M2_LANE == *_api ]]; then
        printf 'API cap: %s microUSD НА RUN; in-flight расходы могут превысить cap. LiteLLM: %s\n' "$M2_SPEND" "$M2_PROXY"
    else
        printf 'Расходуется лимит подписки; денежная стоимость неизвестна.\n'
    fi
    if [[ ${M2_APPROVED:-false} != true ]]; then
        [[ -t 0 ]] || { m2_error 'Для noninteractive запуска требуется --yes.'; return 1; }
        read -r -p 'Запустить реальных Employees с этими настройками? [y/N] ' answer || return 1
        [[ $answer == y || $answer == Y || $answer == да ]] || { m2_error 'Запуск не подтверждён.'; return 1; }
    fi
    # Raw credential bytes are read only after the explicit account/use confirmation.
    if [[ $M2_LANE == codex_cli ]]; then
        m1_open_auth "$M2_AUTH_SOURCE" || return 1
        M2_ACCOUNT=$(jq -er '.tokens.account_id' "/proc/$BASHPID/fd/$M1_AUTH_FD") || return 1
    fi
    m2_json_write "$M2_PRIVATE/config.json" "$(jq -c --arg lane "$M2_LANE" --arg model "$M2_MODEL" \
        --arg auth "$M2_AUTH_SOURCE" --arg account "$M2_ACCOUNT" --arg proxy "$M2_PROXY" \
        --arg master "$M2_PROXY_MASTER" --arg spend "$M2_SPEND" \
        '. + {lane:$lane,model:$model,credential_file:$auth,account_id:(if $lane=="claude_cli" then $account else null end),
            proxy_endpoint:(if $proxy=="" then null else $proxy end),proxy_master_file:(if $master=="" then null else $master end),
            max_spend_microusd:(if $spend=="" then null else ($spend|tonumber) end)}' <<<"$config")"
}
