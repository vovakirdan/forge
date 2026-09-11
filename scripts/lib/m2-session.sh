#!/usr/bin/env bash
# Runtime lifecycle reuses M1's reviewed exact-child and owned-container cleanup.

m2_database_url() {
    local expected
    [[ $M2_DATABASE =~ ^forge_m2_[0-9a-f]{32}$ \
        && ${FORGE_POSTGRES_USER:-} =~ ^[a-z_][a-z0-9_]{0,62}$ \
        && ${FORGE_POSTGRES_DATABASE:-} =~ ^[a-z_][a-z0-9_]{0,62}$ \
        && ${FORGE_POSTGRES_PASSWORD:-} =~ ^[0-9a-f]{48}$ \
        && ${FORGE_POSTGRES_HOST:-} == 127.0.0.1 \
        && ${FORGE_POSTGRES_PORT:-} =~ ^[1-9][0-9]{0,4}$ && $FORGE_POSTGRES_PORT -le 65535 ]] || return 1
    expected="postgresql://${FORGE_POSTGRES_USER}:${FORGE_POSTGRES_PASSWORD}@127.0.0.1:${FORGE_POSTGRES_PORT}/${FORGE_POSTGRES_DATABASE}"
    [[ ${FORGE_DATABASE_URL:-} == "$expected" ]] || return 1
    printf '%s/%s\n' "${expected%/*}" "$M2_DATABASE"
}

m2_prepare() {
    local executable
    for executable in cargo podman podman-compose jq uuidgen timeout install stat flock git; do require_command "$executable"; done
    require_rootless_podman
    [[ -z ${CARGO_TARGET_DIR:-} || $CARGO_TARGET_DIR == target || $CARGO_TARGET_DIR == "$FORGE_ROOT_DIR/target" ]] \
        || { m2_error 'Убери CARGO_TARGET_DIR для этого launcher.'; return 1; }
    [[ $(m2_git --git-dir="$M2_TARGET" symbolic-ref HEAD) == refs/heads/main ]] || return 1
    if [[ -n $(m2_git --git-dir="$M2_TARGET" for-each-ref --format='%(refname)') ]]; then
        m2_error 'Target уже содержит историю. Сессии не сбрасываются: создай другой пустой playground.'; return 1
    fi
    m2_settings || return 1
    M2_ROOT=$(mktemp -d /tmp/fm2.XXXXXXXX)
    M2_HOST="m2-manual-$(m1_uuid7)"
    M2_DATABASE="forge_m2_$(m1_uuid7 | tr -d '-')"
    M2_PROJECT_ID=$(m1_uuid7)
    m2_bind_session
    M1_SESSION_NAME=M2
    install -d -m 700 "$M1_EXEC" "$M2_ROOT/evidence"
    m2_record '{"outcome":"unverified","phase":"preparing"}'
    m2_json_write "$M2_PRIVATE/current.json" "$(jq -cn --arg root "$M2_ROOT" '{root:$root}')"
    if [[ $M2_LANE == codex_cli ]]; then
        M1_AUTH="$M2_ROOT/auth.json"
        m1_copy_auth || return 1
        M2_ENROLL_FILE=$M1_AUTH
    else
        M2_ENROLL_FILE=$M2_AUTH_SOURCE
    fi
    printf 'Приватная сессия: %s\nГотовлю сервисы, бинарники и текущий runtime image…\n' "$M2_ROOT"
    cd "$FORGE_ROOT_DIR"
    export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
    if [[ ! -e $FORGE_DEV_CONFIG_FILE && ! -e $FORGE_DEV_SECRETS_FILE ]]; then
        bash "$SCRIPT_DIR/dev-init.sh" >"$M2_ROOT/setup.log" 2>&1
    fi
    bash "$SCRIPT_DIR/dev-up.sh" >>"$M2_ROOT/setup.log" 2>&1
    require_dev_environment
    cargo build --workspace --bins --locked >>"$M2_ROOT/setup.log" 2>&1
    bash "$SCRIPT_DIR/build-runtime-image.sh" >>"$M2_ROOT/setup.log" 2>&1
    M2_IMAGE="localhost/forge-runtime@$(podman image inspect --format '{{.Digest}}' localhost/forge-runtime:m1)"
    [[ $M2_IMAGE =~ @sha256:[a-f0-9]{64}$ ]] || return 1
    m2_start_services
}

m2_start_services() {
    local binaries="$FORGE_ROOT_DIR/target/debug" database_url postgres deadline
    local -a options=()
    database_url=$(m2_database_url) || { m2_error 'Требуется generated local development database config.'; return 1; }
    postgres=$(service_container_id postgres)
    timeout --kill-after=1 15 podman exec "$postgres" createdb --username "$FORGE_POSTGRES_USER" -- "$M2_DATABASE" \
        >"$M2_ROOT/database.log" 2>&1
    FORGE_DATABASE_URL="$database_url" timeout --kill-after=1 60 "$binaries/forge-migrate" >"$M2_ROOT/migrate.log" 2>&1
    timeout --kill-after=1 15 "$binaries/forge-secret-store" --directory "$M2_ROOT/secrets" --initialize file \
        >"$M2_ROOT/secret-store.log" 2>&1
    if [[ $M2_LANE == *_api ]]; then
        options+=(--litellm-url "$M2_PROXY" --litellm-master-key "$M2_PROXY_MASTER")
    fi
    FORGE_DATABASE_URL="$database_url" "$binaries/forge-core" \
        --api-socket "$M2_ROOT/api.sock" --supervisor-socket "$M2_ROOT/supervisor.sock" \
        --execution-root "$M1_EXEC" --secret-store "$M2_ROOT/secrets" \
        --observability-address 127.0.0.1:0 "${options[@]}" >"$M2_ROOT/core.log" 2>&1 &
    M1_CORE_PID=$!
    M1_CORE_TOKEN=$(m1_process_token "$M1_CORE_PID") || return 1
    m2_record "$(jq -cn --argjson pid "$M1_CORE_PID" --arg token "$M1_CORE_TOKEN" '{core_pid:$pid,core_token:$token}')"
    deadline=$((SECONDS + 20))
    until timeout --kill-after=1 2 "$binaries/forge-cli" --socket "$M2_ROOT/api.sock" get /v1/health >/dev/null 2>&1; do
        ((SECONDS < deadline)) && m1_process_token "$M1_CORE_PID" >/dev/null \
            || { m2_error 'Core не запустился; смотри core.log.'; return 1; }
        sleep 0.2
    done
    "$binaries/forge-supervisor" --state-directory "$M1_EXEC" --host-id "$M2_HOST" \
        --socket "$M2_ROOT/supervisor.sock" --observability-address 127.0.0.1:0 \
        >"$M2_ROOT/supervisor.log" 2>&1 &
    M1_SUPERVISOR_PID=$!
    M1_SUPERVISOR_TOKEN=$(m1_process_token "$M1_SUPERVISOR_PID") || return 1
    m2_record "$(jq -cn --argjson pid "$M1_SUPERVISOR_PID" --arg token "$M1_SUPERVISOR_TOKEN" '{supervisor_pid:$pid,supervisor_token:$token}')"
}

# This override affects only this launcher process, never run-m1.
m1_stop_project() {
    local M1_COMMAND_DEADLINE=$((SECONDS + 12))
    m2_command stop_project_execution '{"reason":"M2 operator exercise managed stop"}' >/dev/null
}

m2_on_exit() {
    local workflow_status=$? status cleanup=0 evidence_collection=unverified
    status=$workflow_status
    trap - EXIT
    trap '' INT TERM
    m1_close_auth
    if [[ -n ${M2_ROOT:-} ]]; then
        if [[ -n ${M2_STARTED:-} ]]; then
            m1_stop_project || cleanup=1
            m2_await_stopped || cleanup=1
            if m2_collect; then evidence_collection=passed
            else evidence_collection=failed; fi
        fi
        if [[ -n ${M1_CORE_PID:-} || -n ${M1_SUPERVISOR_PID:-} ]]; then
            m1_cleanup || cleanup=1
        fi
        if [[ -n ${M1_AUTH:-} && -f $M1_AUTH ]]; then m1_remove_import_copy || cleanup=1; fi
        if ((cleanup != 0)) || [[ $evidence_collection == failed ]]; then
            ((status != 0)) || status=1
        fi
        if ! m2_finish_report "$workflow_status" "$cleanup" "$evidence_collection"; then
            ((status != 0)) || status=1
        fi
        printf 'Evidence: %s/evidence/report.json\nСессия, DB, Git и логи сохранены: %s\n' "$M2_ROOT" "$M2_ROOT"
    fi
    exit "$status"
}

m2_await_stopped() {
    local deadline=$((SECONDS + 45)) runs id detail complete
    while ((SECONDS < deadline)); do
        runs=$(m2_pages "/v1/projects/$M2_PROJECT_ID/runs") || return 1
        if jq -e 'all(.items[]; .observed_state == "stopped" or .observed_state == "failed")' <<<"$runs" >/dev/null \
            && m1_scan_owned check; then
            complete=true
            while IFS= read -r id; do
                detail=$(m2_read "/v1/projects/$M2_PROJECT_ID/runs/$id") || return 1
                jq -e '.diagnostics.runtime_report!=null' <<<"$detail" >/dev/null || complete=false
            done < <(jq -r '.items[].id' <<<"$runs")
            [[ $complete != true ]] || return 0
        fi
        sleep 0.5
    done
    m2_error 'Нормальная остановка не подтверждена; точная аварийная очистка продолжится.'
}

m2_stop_external() {
    local pid token current deadline
    m2_load_session || return 1
    pid=$(jq -er '.launcher_pid' "$M2_ROOT/session.json")
    token=$(jq -er '.launcher_token' "$M2_ROOT/session.json")
    if current=$(m1_process_token "$pid"); then
        [[ $current == "$token" ]] || { m2_error 'PID launcher уже принадлежит другому процессу.'; return 1; }
        # Its EXIT trap owns children and can prove their exact lifetime identity.
        m1_stop_project || true
        kill -INT "$pid" || return 1
        deadline=$((SECONDS + 90))
        while current=$(m1_process_token "$pid"); do
            [[ $current == "$token" ]] || return 1
            ((SECONDS < deadline)) || { m2_error 'Остановка ещё не завершена; смотри приватные логи.'; return 1; }
            sleep 0.5
        done
        m1_scan_owned check || return 1
        printf 'Сессия остановлена; evidence: %s/evidence/report.json\n' "$M2_ROOT"
    else
        if [[ $(jq -r '.phase' "$M2_ROOT/session.json") == closed ]] && m1_scan_owned check; then
            printf 'Сессия уже остановлена; evidence: %s/evidence/report.json\n' "$M2_ROOT"
            return 0
        fi
        # No guessed PID signalling or broad podman stop when the owner vanished.
        m1_stop_project || true
        m1_scan_owned stop || return 1
        m1_scan_owned check || return 1
        m2_error 'Launcher отсутствует. Session containers остановлены; статус unverified, проверь retained daemons вручную.'
    fi
}

m2_main() {
    local action=${1:---help} option
    (($# == 0)) || shift
    M2_PROJECT_DIR=$PWD M2_APPROVED=false M2_ROOT='' M2_STARTED=''
    while (($#)); do
        option=$1
        case "$option" in
            --yes) M2_APPROVED=true; shift ;;
            --project-dir|--lane|--model|--auth-file|--account-id|--proxy-endpoint|--proxy-master-file|--max-spend-microusd)
                (($# >= 2)) || { m2_usage >&2; return 2; }
                case "$option" in
                    --project-dir) M2_PROJECT_DIR=$2 ;; --lane) M2_LANE=$2 ;; --model) M2_MODEL=$2 ;;
                    --auth-file) M2_AUTH_SOURCE=$2 ;; --account-id) M2_ACCOUNT=$2 ;;
                    --proxy-endpoint) M2_PROXY=$2 ;; --proxy-master-file) M2_PROXY_MASTER=$2 ;;
                    --max-spend-microusd) M2_SPEND=$2 ;;
                esac
                shift 2 ;;
            *) m2_usage >&2; return 2 ;;
        esac
    done
    case "$action" in --help|-h|help) m2_usage; return ;; init|run|status|stop|evidence) ;; *) m2_usage >&2; return 2 ;; esac
    M2_PROJECT_DIR=$(CDPATH= cd -- "$M2_PROJECT_DIR" && pwd -P) || return 1
    [[ $M2_PROJECT_DIR != *$'\n'* && $M2_PROJECT_DIR != *$'\r'* ]] || return 1
    M2_PRIVATE="$M2_PROJECT_DIR/.forge-m2" M2_TARGET="$M2_PROJECT_DIR/.forge-m2/target.git"
    [[ $action != init ]] || { m2_init; return; }
    m2_private_directory "$M2_PRIVATE" || return 1
    case "$action" in
        status) m2_status ;;
        evidence) m2_evidence_command ;;
        stop) m2_stop_external ;;
        run)
            [[ ! -L $M2_PRIVATE/run.lock ]] || return 1
            exec {M2_LOCK_FD}>"$M2_PRIVATE/run.lock"
            flock --nonblock "$M2_LOCK_FD" || { m2_error 'Этот playground уже запускается.'; return 1; }
            trap m2_on_exit EXIT
            trap 'exit 130' INT
            trap 'exit 143' TERM
            m2_prepare
            m2_seed
            m2_workflow ;;
    esac
}
