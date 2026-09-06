#!/usr/bin/env bash
# Developer convenience entrypoint; not an unattended installer or acceptance gate.
set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
source "$SCRIPT_DIR/lib/m1-input.sh"
source "$SCRIPT_DIR/lib/m1-commands.sh"
source "$SCRIPT_DIR/lib/m1-session.sh"

usage() {
    printf '%s\n' \
        'Usage: just run-m1 [--auth-file PATH] [--model MODEL] [--task TEXT] [--yes]' \
        'Interactive: confirm current Codex auth, model and task; Forge handles setup and stop.' \
        '--yes explicitly approves subscription usage; noninteractive mode requires all three values.' \
        'No login, mutation of the source auth, host config import, or automatic provider fallback.'
}

m1_reset_launcher_state
while (($#)); do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --yes) M1_APPROVED=true; shift ;;
        --auth-file|--model|--task)
            (($# >= 2)) || { usage >&2; exit 2; }
            case "$1" in
                --auth-file) M1_AUTH_SOURCE="$2" ;;
                --model) M1_MODEL="$2" ;;
                --task) M1_TASK_TEXT="$2" ;;
            esac
            shift 2 ;;
        *) usage >&2; exit 2 ;;
    esac
done

on_exit() {
    local status=$?
    trap - EXIT
    # Repeated Ctrl-C cannot skip cleanup once paid execution might exist.
    trap '' INT TERM
    m1_close_auth
    if [[ -n ${M1_ROOT:-} ]] && ! m1_cleanup; then
        [[ "$status" != 0 ]] || status=1
    fi
    if [[ -n ${M1_AUTH:-} && ( -e "$M1_AUTH" || -L "$M1_AUTH" ) ]]; then
        if ! m1_remove_import_copy; then
            printf 'Не удалось безопасно удалить временную копию auth; сохрани приватность каталога сессии.\n' >&2
            [[ "$status" != 0 ]] || status=1
        fi
    fi
    exit "$status"
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for executable in cargo podman podman-compose jq uuidgen timeout install stat awk; do
    require_command "$executable"
done
require_rootless_podman
[[ -z ${CARGO_TARGET_DIR:-} || "$CARGO_TARGET_DIR" == target \
    || "$CARGO_TARGET_DIR" == "$FORGE_ROOT_DIR/target" ]] || fail 'unset CARGO_TARGET_DIR for this local launcher'
m1_prepare_inputs

readonly M1_BASE="$HOME/.forge-m1"
if [[ -e "$M1_BASE" ]]; then
    [[ -d "$M1_BASE" && ! -L "$M1_BASE" \
        && $(stat -c '%u:%a' "$M1_BASE") == "$(id -u):700" ]] || fail 'existing ~/.forge-m1 must be a private owned directory, not a symlink'
else
    install -d -m 700 "$M1_BASE"
fi
M1_ROOT=$(mktemp -d "$M1_BASE/run.XXXXXXXX")
M1_EXEC="$M1_ROOT/exec"
M1_AUTH="$M1_ROOT/auth.json"
M1_HOST="m1-manual-$(m1_uuid7)"
M1_DATABASE="forge_m1_$(m1_uuid7 | tr -d '-')"
# Match Core sockaddr_un validation before bootstrapping any services or DB.
[[ ${#M1_EXEC} -le 49 ]] || fail 'home path is too long for M1 Unix sockets; use the documented explicit-root setup'
printf 'Приватная сессия и логи: %s\n' "$M1_ROOT"
install -d -m 700 "$M1_EXEC"
m1_copy_auth

cd "$FORGE_ROOT_DIR"
printf 'Подготовка локальных сервисов и бинарников…\n'
if [[ ! -e "$FORGE_DEV_CONFIG_FILE" && ! -e "$FORGE_DEV_SECRETS_FILE" ]]; then
    bash "$SCRIPT_DIR/dev-init.sh" >"$M1_ROOT/setup.log" 2>&1
fi
bash "$SCRIPT_DIR/dev-up.sh" >>"$M1_ROOT/setup.log" 2>&1
require_dev_environment
cargo build --workspace --bins --locked >>"$M1_ROOT/setup.log" 2>&1
# Rebuild through the existing cached path so a stale runner is never selected.
bash "$SCRIPT_DIR/build-runtime-image.sh" >>"$M1_ROOT/setup.log" 2>&1
M1_IMAGE="localhost/forge-runtime@$(podman image inspect --format '{{.Digest}}' localhost/forge-runtime:m1)"
[[ "$M1_IMAGE" =~ @sha256:[a-f0-9]{64}$ ]] || fail 'runtime image digest unavailable'
m1_start_services
m1_seed
m1_remove_import_copy
printf 'Project: %s\nTask: %s\nЗапускаю Codex. Ctrl-C запросит остановку, сохранив файлы и логи.\n' \
    "$M1_PROJECT_ID" "$M1_TASK_ID"
m1_start_project

m1_monitor() {
    local deadline=$((SECONDS + 660)) state previous='' view runs run_id detail
    while ((SECONDS < deadline)); do
        view=$(m1_cli get "/v1/projects/$M1_PROJECT_ID/tasks/$M1_TASK_ID") || return
        state=$(jq -er '.lifecycle' <<<"$view") || return
        if [[ "$state" != "$previous" ]]; then
            printf 'Task: %s\n' "$state"
            previous="$state"
        fi
        case "$state" in
            done) m1_show_result; return ;;
            cancelled|waiting)
                m1_show_result
                printf 'Задача не завершена успешно; проверь причину ожидания и diagnostics.\n' >&2
                return 1 ;;
        esac
        # Physical exit is evidence, never a fabricated successful Task outcome.
        runs=$(m1_cli get "/v1/projects/$M1_PROJECT_ID/runs") || return
        run_id=$(jq -r --arg task "$M1_TASK_ID" '.items[] | select(.task_id == $task) | .id' <<<"$runs")
        if [[ -n "$run_id" ]]; then
            detail=$(m1_cli get "/v1/projects/$M1_PROJECT_ID/runs/$run_id") || return
            if jq -e '.diagnostics.runtime_report != null' <<<"$detail" >/dev/null; then
                # Allow Core a short interval to commit its final lifecycle projection.
                sleep 1
                state=$(m1_cli get "/v1/projects/$M1_PROJECT_ID/tasks/$M1_TASK_ID" | jq -er '.lifecycle') || return
                m1_show_result
                [[ "$state" == done ]] && return 0
                printf 'Codex вышел без принятого результата Task; смотри diagnostics.\n' >&2
                return 1
            fi
        fi
        sleep 2
    done
    printf 'Истёк срок ожидания; запрашиваю остановку.\n' >&2
    return 1
}

m1_monitor
