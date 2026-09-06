#!/usr/bin/env bash

# Sourced by run-m1.sh after its private session and development env are ready.
M1_LAUNCHER_PID=$BASHPID

m1_session_error() {
    printf 'forge M1: %s\n' "$*" >&2
    if [[ -n "${M1_ROOT:-}" && -d "$M1_ROOT" && ! -L "$M1_ROOT/cleanup.log" ]]; then
        printf 'forge M1: %s\n' "$*" >>"$M1_ROOT/cleanup.log" || true
    fi
    return 1
}

# A PID alone can be reused. Only signal the exact direct child we launched.
m1_process_token() {
    local record
    local -a fields
    [[ "$1" =~ ^[1-9][0-9]*$ ]] || return 1
    IFS= read -r record 2>/dev/null <"/proc/$1/stat" || return 1
    read -r -a fields <<<"${record##*) }"
    [[ "${fields[0]}" != Z && "${fields[0]}" != X ]] || return 1
    printf '%s:%s\n' "${fields[1]}" "${fields[19]}"
}

m1_start_services() {
    local binaries="$FORGE_ROOT_DIR/target/debug" postgres database_url deadline
    [[ "$M1_DATABASE" =~ ^forge_m1_[0-9a-f]{32}$ ]] \
        || { m1_session_error 'invalid dedicated database name'; return 1; }
    [[ "${FORGE_DATABASE_URL##*/}" == "$FORGE_POSTGRES_DATABASE" ]] \
        || { m1_session_error 'expected the generated development database URL'; return 1; }
    [[ "$M1_EXEC" == "$M1_ROOT/exec" && "$M1_ROOT" == /* && ! -L "$M1_EXEC" ]] \
        || { m1_session_error 'invalid private execution directory'; return 1; }
    umask 077
    mkdir -m 700 -p -- "$M1_EXEC" || return 1
    M1_API_SOCKET="$M1_ROOT/api.sock"
    M1_SUPERVISOR_SOCKET="$M1_ROOT/supervisor.sock"
    postgres="$(service_container_id postgres)" || return 1
    timeout --kill-after=1 15 podman exec "$postgres" createdb \
        --username "$FORGE_POSTGRES_USER" "$M1_DATABASE" \
        >"$M1_ROOT/database.log" 2>&1 \
        || { m1_session_error 'dedicated database creation failed; see database.log'; return 1; }
    database_url="${FORGE_DATABASE_URL%/*}/$M1_DATABASE"
    # The URL is inherited only; suppress connection errors that may echo it.
    FORGE_DATABASE_URL="$database_url" timeout --kill-after=1 60 \
        "$binaries/forge-migrate" >"$M1_ROOT/migrate.log" 2>/dev/null \
        || { m1_session_error 'dedicated database migration failed'; return 1; }
    timeout --kill-after=1 15 "$binaries/forge-secret-store" \
        --directory "$M1_ROOT/secrets" --initialize file \
        >"$M1_ROOT/secret-store.log" 2>&1 \
        || { m1_session_error 'private Secret Store initialization failed'; return 1; }
    FORGE_DATABASE_URL="$database_url" "$binaries/forge-core" \
        --api-socket "$M1_API_SOCKET" --supervisor-socket "$M1_SUPERVISOR_SOCKET" \
        --execution-root "$M1_EXEC" --secret-store "$M1_ROOT/secrets" \
        --observability-address 127.0.0.1:0 >"$M1_ROOT/core.log" 2>&1 &
    M1_CORE_PID=$!
    M1_CORE_TOKEN="$(m1_process_token "$M1_CORE_PID")" || M1_CORE_TOKEN=''
    deadline=$((SECONDS + 20))
    while ((SECONDS < deadline)); do
        if timeout --kill-after=1 2 "$binaries/forge-cli" --socket "$M1_API_SOCKET" \
            get /v1/health >/dev/null 2>&1; then
            break
        fi
        m1_process_token "$M1_CORE_PID" >/dev/null \
            || { m1_session_error 'Core exited before API readiness; see core.log'; return 1; }
        sleep 0.2
    done
    ((SECONDS < deadline)) \
        || { m1_session_error 'Core API was not ready within 20 seconds'; return 1; }
    "$binaries/forge-supervisor" --state-directory "$M1_EXEC" --host-id "$M1_HOST" \
        --socket "$M1_SUPERVISOR_SOCKET" --observability-address 127.0.0.1:0 \
        >"$M1_ROOT/supervisor.log" 2>&1 &
    M1_SUPERVISOR_PID=$!
    M1_SUPERVISOR_TOKEN="$(m1_process_token "$M1_SUPERVISOR_PID")" || M1_SUPERVISOR_TOKEN=''
    [[ -n "$M1_SUPERVISOR_TOKEN" ]] \
        || { m1_session_error 'Supervisor exited during startup; see supervisor.log'; return 1; }
}

m1_stop_process() {
    local prefix="$1" grace="${2:-10}" pid_name token_name pid token current deadline forced=0
    pid_name="${prefix}_PID"
    token_name="${prefix}_TOKEN"
    pid="${!pid_name:-}"
    token="${!token_name:-}"
    [[ -n "$pid" ]] || return 0
    if ! current="$(m1_process_token "$pid")"; then
        m1_join_process "$pid" "$prefix"
        return "$?"
    fi
    [[ "$current" == "$token" && "${current%%:*}" == "$M1_LAUNCHER_PID" ]] \
        || { m1_session_error "$prefix process ownership could not be verified"; return 1; }
    kill -INT "$pid" 2>/dev/null || true
    deadline=$((SECONDS + grace))
    while current="$(m1_process_token "$pid")"; do
        [[ "$current" == "$token" ]] \
            || { m1_session_error "$prefix process identity changed"; return 1; }
        ((SECONDS < deadline)) || break
        sleep 0.1
    done
    if current="$(m1_process_token "$pid")"; then
        [[ "$current" == "$token" ]] || return 1
        forced=1
        m1_session_error "$prefix required forced termination" || true
        kill -KILL "$pid" 2>/dev/null || true
        deadline=$((SECONDS + 2))
        while m1_process_token "$pid" >/dev/null; do
            ((SECONDS < deadline)) \
                || { m1_session_error "$prefix is still alive after termination"; return 1; }
            sleep 0.1
        done
    fi
    m1_join_process "$pid" "$prefix" || return 1
    return "$forced"
}

m1_join_process() {
    local status=0
    wait "$1" 2>/dev/null || status=$?
    ((status == 0)) || m1_session_error "$2 exited with status $status"
}

m1_podman() {
    timeout --kill-after=1 8 podman "$@" 2>/dev/null
}

# Inspect only labels, mounts and state; never capture environment credentials.
m1_container_state() {
    local snapshot
    snapshot="$(m1_podman inspect --format \
        '{"labels":{{json .Config.Labels}},"mounts":{{json .Mounts}},"running":{{json .State.Running}}}' "$1")" \
        || return 1
    ((${#snapshot} <= 65536)) || return 1
    jq -er --arg host "$M1_HOST" --arg root "$M1_EXEC/runtime/" '
        def epoch:
            test("^[1-9][0-9]*$") and
            (length < 20 or (length == 20 and . <= "18446744073709551615"));
        def owned_mount:
            .Destination == "/run/forge" and (.Source | type == "string") and
            (.Source | startswith($root)) and
            (.Source | ltrimstr($root) | split("/") |
                length == 2 and
                (.[0] | test("^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")) and
                (.[1] | epoch));
        if (.labels | type) != "object" or (.mounts | type) != "array" or
           (.running | type) != "boolean" then error("invalid inspection")
        elif .labels["forge.managed"] != "true" or .labels["forge.host"] != $host or
             ([.mounts[] | owned_mount] | any | not) then "foreign"
        elif .running then "running" else "stopped" end
    ' <<<"$snapshot" 2>/dev/null
}

# Return 0 for proven quiet, 2 if an owned runtime was live, 1 on inspection error.
m1_scan_owned() {
    local action="$1" ids id state failed=0 live=0 deadline=$((SECONDS + 20))
    ids="$(m1_podman ps --all --no-trunc --filter label=forge.managed=true \
        --filter "label=forge.host=$M1_HOST" --format '{{.ID}}')" \
        || { m1_session_error 'cannot inventory session containers'; return 1; }
    ((${#ids} <= 65536)) || return 1
    while IFS= read -r id; do
        [[ -n "$id" ]] || continue
        if [[ ! "$id" =~ ^[0-9a-fA-F]{64}$ ]]; then
            m1_session_error 'container inventory returned an invalid full ID' || true
            failed=1
            continue
        fi
        if ((SECONDS >= deadline)); then
            m1_session_error 'container cleanup exceeded its bounded inventory window' || true
            return 1
        fi
        if ! state="$(m1_container_state "$id")"; then
            m1_session_error "cannot inspect container $id" || true
            failed=1
            continue
        fi
        [[ "$state" == running ]] || continue
        live=2
        [[ "$action" == stop ]] || continue
        m1_session_error "emergency stop required for exact session container $id" || true
        m1_podman stop --time 3 "$id" >/dev/null || true
        if [[ "$(m1_container_state "$id")" != stopped ]]; then
            m1_podman kill --signal KILL "$id" >/dev/null || true
        fi
        if [[ "$(m1_container_state "$id")" != stopped ]]; then
            m1_session_error "physical stop could not be proved for $id" || true
            failed=1
        fi
    done <<<"$ids"
    ((failed == 0)) || return 1
    return "$live"
}

m1_cleanup() {
    # EXIT traps inherited by command substitutions must not stop their parent.
    [[ "$BASHPID" == "$M1_LAUNCHER_PID" ]] || return 0
    [[ "${M1_CLEANUP_DONE:-0}" == 0 ]] || return "${M1_CLEANUP_STATUS:-0}"
    M1_CLEANUP_DONE=1
    M1_CLEANUP_STATUS=0
    [[ -n "${M1_ROOT:-}" ]] || return 0
    local deadline scan
    printf 'forge M1: stopping this session; retained files: %q\n' "$M1_ROOT" >&2
    if [[ -n "${M1_PROJECT_ID:-}" ]]; then
        if ! m1_stop_project; then
            m1_session_error 'named project stop failed; continuing independent physical cleanup' || true
            M1_CLEANUP_STATUS=1
        fi
        deadline=$((SECONDS + 10))
        while ((SECONDS < deadline)); do
            if m1_scan_owned check; then
                break
            else
                scan=$?
                if [[ "$scan" != 2 ]]; then
                    M1_CLEANUP_STATUS=1
                    break
                fi
            fi
            sleep 0.2
        done
    fi
    # End dispatch before the final inventory, including unknown/Created Runs.
    m1_stop_process M1_SUPERVISOR || M1_CLEANUP_STATUS=1
    if [[ -n "${M1_HOST:-}" && -n "${M1_EXEC:-}" ]]; then
        m1_scan_owned stop || M1_CLEANUP_STATUS=1
        # Core's evidence worker ticks every five seconds. Keep it alive while
        # issued Podman operations settle, then independently check once more.
        sleep 6
        m1_scan_owned stop || M1_CLEANUP_STATUS=1
    elif [[ -n "${M1_SUPERVISOR_PID:-}" ]]; then
        m1_session_error 'session identity missing; physical quiescence is unproven' || true
        M1_CLEANUP_STATUS=1
    fi
    m1_stop_process M1_CORE || M1_CLEANUP_STATUS=1
    if ((M1_CLEANUP_STATUS != 0)); then
        printf 'forge M1: cleanup needed force or could not prove a clean stop; inspect %q\n' "$M1_ROOT" >&2
    else
        printf 'forge M1: session processes stopped; no running session containers remain.\n' >&2
    fi
    return "$M1_CLEANUP_STATUS"
}
