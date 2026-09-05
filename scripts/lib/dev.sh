#!/usr/bin/env bash

set -euo pipefail

readonly FORGE_SCRIPT_LIB_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly FORGE_ROOT_DIR="$(CDPATH= cd -- "$FORGE_SCRIPT_LIB_DIR/../.." && pwd -P)"
readonly FORGE_DEV_DIR="$FORGE_ROOT_DIR/var/dev"
readonly FORGE_DEV_CONFIG_FILE="$FORGE_DEV_DIR/config.env"
readonly FORGE_DEV_SECRETS_FILE="$FORGE_DEV_DIR/secrets.env"
readonly FORGE_COMPOSE_FILE="$FORGE_ROOT_DIR/infra/dev/compose.yaml"

fail() {
    printf 'forge dev: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

require_rootless_podman() {
    local rootless

    rootless="$(podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null)" \
        || fail "cannot inspect the Podman security mode"
    [[ "$rootless" == "true" ]] \
        || fail "Forge development services require rootless Podman"
}

require_dev_environment() {
    [[ -f "$FORGE_DEV_CONFIG_FILE" ]] || fail "run `just dev-init` first"
    [[ -f "$FORGE_DEV_SECRETS_FILE" ]] || fail "run `just dev-init` first"
    [[ ! -L "$FORGE_DEV_CONFIG_FILE" ]] || fail "refusing a symlinked development config file"
    [[ ! -L "$FORGE_DEV_SECRETS_FILE" ]] || fail "refusing a symlinked development secrets file"

    set -a
    # Both files are generated locally with mode 0600 and contain shell-safe values only.
    # shellcheck disable=SC1090
    source "$FORGE_DEV_CONFIG_FILE"
    # shellcheck disable=SC1090
    source "$FORGE_DEV_SECRETS_FILE"
    set +a
}

compose() {
    local output
    local status

    require_command podman
    require_command podman-compose
    require_rootless_podman
    require_dev_environment

    # podman-compose 1.0.x logs the complete `podman run` command, including
    # environment values. Keep its operational output useful without exposing
    # locally generated credentials to the terminal or CI logs.
    if output="$(podman-compose \
        --file "$FORGE_COMPOSE_FILE" \
        --project-name "$FORGE_COMPOSE_PROJECT_NAME" \
        "$@" 2>&1)"; then
        status=0
    else
        status=$?
    fi

    printf '%s\n' "$output" | sed \
        -e "s/${FORGE_POSTGRES_PASSWORD}/[redacted]/g" \
        -e "s/${FORGE_NATS_AUTH_TOKEN}/[redacted]/g" \
        -e "s/${FORGE_REDIS_PASSWORD}/[redacted]/g" \
        -e "s/${FORGE_MINIO_ROOT_PASSWORD}/[redacted]/g"
    return "$status"
}

service_container_id() {
    local service="$1"
    local container_id

    container_id="$(podman ps \
        --filter "label=io.podman.compose.project=$FORGE_COMPOSE_PROJECT_NAME" \
        --filter "label=com.docker.compose.service=$service" \
        --format '{{.ID}}' \
        | awk 'NF { print; exit }')"
    [[ -n "$container_id" ]] || fail "container for service '$service' is not running"
    printf '%s\n' "$container_id"
}

wait_for_postgres() {
    local container_id
    local attempt=0

    container_id="$(service_container_id postgres)"
    while ((attempt < 60)); do
        if podman exec "$container_id" pg_isready -q -U "$FORGE_POSTGRES_USER" -d "$FORGE_POSTGRES_DATABASE"; then
            return 0
        fi

        sleep 1
        attempt=$((attempt + 1))
    done

    fail "PostgreSQL did not become ready within 60 seconds"
}

wait_for_nats() {
    local attempt=0
    local health_url="http://${FORGE_NATS_HOST}:${FORGE_NATS_MONITOR_PORT}/healthz?js-enabled-only=true"

    require_command curl
    while ((attempt < 60)); do
        if curl --fail --silent --show-error --max-time 2 "$health_url" >/dev/null; then
            return 0
        fi

        sleep 1
        attempt=$((attempt + 1))
    done

    fail "NATS JetStream did not become ready within 60 seconds"
}

wait_for_redis() {
    local container_id
    local attempt=0

    container_id="$(service_container_id redis)"
    while ((attempt < 60)); do
        if printf 'AUTH %s\nPING\n' "$FORGE_REDIS_PASSWORD" \
            | podman exec -i "$container_id" redis-cli --no-auth-warning --raw 2>/dev/null \
            | tail -n 1 \
            | grep -qx 'PONG'; then
            return 0
        fi

        sleep 1
        attempt=$((attempt + 1))
    done

    fail "Redis did not become ready within 60 seconds"
}

wait_for_minio() {
    local attempt=0
    local health_url="${FORGE_MINIO_ENDPOINT}/minio/health/ready"

    require_command curl
    while ((attempt < 60)); do
        if curl --fail --silent --show-error --max-time 2 "$health_url" >/dev/null; then
            return 0
        fi

        sleep 1
        attempt=$((attempt + 1))
    done

    fail "MinIO did not become ready within 60 seconds"
}
