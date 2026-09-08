#!/usr/bin/env bash
# Private deterministic demo helpers. Never discover provider credentials or
# reuse an existing database, Supervisor journal, or execution directory.

m0_demo_error() {
    printf 'forge M0: %s\n' "$*" >&2
    return 1
}

# Return the URL only to the caller's variable; never print it as diagnostics.
# Deliberately accept only the generated local development URL, not arbitrary
# URI syntax whose password/path/query could make suffix substitution unsafe.
m0_demo_database_url() {
    local requested_database="$1" expected_url
    [[ "$requested_database" =~ ^forge_m0_[0-9a-f]{32}$ ]] \
        || { m0_demo_error 'invalid dedicated database name'; return 1; }
    [[ ${FORGE_POSTGRES_USER:-} =~ ^[a-z_][a-z0-9_]{0,62}$ \
        && ${FORGE_POSTGRES_DATABASE:-} =~ ^[a-z_][a-z0-9_]{0,62}$ \
        && ${FORGE_POSTGRES_PASSWORD:-} =~ ^[0-9a-f]{48}$ \
        && ${FORGE_POSTGRES_HOST:-} == 127.0.0.1 \
        && ${FORGE_POSTGRES_PORT:-} =~ ^[1-9][0-9]{0,4}$ ]] \
        || { m0_demo_error 'expected generated local development connection settings'; return 1; }
    ((FORGE_POSTGRES_PORT <= 65535)) \
        || { m0_demo_error 'invalid development PostgreSQL port'; return 1; }
    expected_url="postgresql://${FORGE_POSTGRES_USER}:${FORGE_POSTGRES_PASSWORD}@${FORGE_POSTGRES_HOST}:${FORGE_POSTGRES_PORT}/${FORGE_POSTGRES_DATABASE}"
    [[ ${FORGE_DATABASE_URL:-} == "$expected_url" ]] \
        || { m0_demo_error 'expected the exact generated development database URL'; return 1; }
    printf '%s/%s\n' "${expected_url%/*}" "$requested_database"
}

# A PID alone may be reused. Parent identity and Linux starttime must both match.
m0_demo_process_token() {
    local record
    local -a fields
    [[ "$1" =~ ^[1-9][0-9]*$ ]] || return 1
    IFS= read -r record 2>/dev/null <"/proc/$1/stat" || return 1
    read -r -a fields <<<"${record##*) }"
    [[ ${fields[0]} != Z && ${fields[0]} != X ]] || return 1
    printf '%s:%s\n' "${fields[1]}" "${fields[19]}"
}

m0_demo_stop_process() {
    local pid="$1" token="$2" grace="${3:-5}" current deadline forced=0 status=0
    [[ -n "$pid" ]] || return 0
    if current="$(m0_demo_process_token "$pid")"; then
        [[ "$current" == "$token" && ${current%%:*} == "$M0_DEMO_LAUNCHER_PID" ]] \
            || { m0_demo_error 'process ownership could not be verified'; return 1; }
        # Both native daemons handle SIGINT; TERM would bypass their async drain.
        kill -INT "$pid" 2>/dev/null || true
        deadline=$((SECONDS + grace))
        while current="$(m0_demo_process_token "$pid")"; do
            [[ "$current" == "$token" ]] \
                || { m0_demo_error 'process identity changed'; return 1; }
            ((SECONDS < deadline)) || break
            sleep 0.1
        done
        if current="$(m0_demo_process_token "$pid")"; then
            [[ "$current" == "$token" ]] || return 1
            forced=1
            m0_demo_error 'demo process required forced termination' || true
            kill -KILL "$pid" 2>/dev/null || true
            deadline=$((SECONDS + 2))
            while m0_demo_process_token "$pid" >/dev/null; do
                ((SECONDS < deadline)) \
                    || { m0_demo_error 'demo process is still alive after termination'; return 1; }
                sleep 0.1
            done
        fi
    fi
    wait "$pid" 2>/dev/null || status=$?
    ((status == 0 && forced == 0)) \
        || { m0_demo_error 'demo process did not stop cleanly'; return 1; }
}

m0_demo_cleanup() {
    # An inherited EXIT trap in a command substitution cannot stop its parent.
    [[ "$BASHPID" == "$M0_DEMO_LAUNCHER_PID" ]] || return 0
    [[ ${M0_DEMO_CLEANUP_DONE:-0} == 0 ]] || return "${M0_DEMO_CLEANUP_STATUS:-0}"
    M0_DEMO_CLEANUP_DONE=1
    M0_DEMO_CLEANUP_STATUS=0
    m0_demo_stop_process "${M0_DEMO_SUPERVISOR_PID:-}" "${M0_DEMO_SUPERVISOR_TOKEN:-}" \
        || M0_DEMO_CLEANUP_STATUS=1
    m0_demo_stop_process "${M0_DEMO_CORE_PID:-}" "${M0_DEMO_CORE_TOKEN:-}" \
        || M0_DEMO_CLEANUP_STATUS=1
    # The database and all private files are evidence, not disposable cleanup.
    if [[ -n ${M0_DEMO_ROOT:-} ]]; then
        printf 'forge M0: retained logs/state: %s; database: %s\n' \
            "$M0_DEMO_ROOT" "${M0_DEMO_DATABASE:-not-created}" >&2
    fi
    return "$M0_DEMO_CLEANUP_STATUS"
}
