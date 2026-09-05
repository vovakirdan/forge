#!/usr/bin/env bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

random_hex() {
    od -An -N24 -tx1 /dev/urandom | tr -d ' \n'
}

require_command podman
require_command podman-compose
require_rootless_podman
"$SCRIPT_DIR/dev-init.sh"
require_dev_environment

readonly POSTGRES_CONTAINER_ID="$(service_container_id postgres)"
readonly NEW_POSTGRES_PASSWORD="$(random_hex)"
readonly NEW_NATS_AUTH_TOKEN="$(random_hex)"
readonly SECRETS_TMP_FILE="$(mktemp "$FORGE_DEV_DIR/.secrets.XXXXXX")"

cleanup() {
    rm -f "$SECRETS_TMP_FILE"
}
trap cleanup EXIT

# The generated value is hexadecimal. Supplying this SQL through stdin keeps it
# out of process arguments and terminal output; the local container uses its
# already-authenticated Unix socket for this development-only role change.
printf "ALTER USER %s PASSWORD '%s';\n" "$FORGE_POSTGRES_USER" "$NEW_POSTGRES_PASSWORD" \
    | podman exec -i "$POSTGRES_CONTAINER_ID" \
        psql --set ON_ERROR_STOP=1 --username "$FORGE_POSTGRES_USER" --dbname "$FORGE_POSTGRES_DATABASE" \
        >/dev/null

{
    printf 'FORGE_POSTGRES_PASSWORD=%s\n' "$NEW_POSTGRES_PASSWORD"
    printf 'FORGE_NATS_AUTH_TOKEN=%s\n' "$NEW_NATS_AUTH_TOKEN"
    printf 'FORGE_REDIS_PASSWORD=%s\n' "$FORGE_REDIS_PASSWORD"
    printf 'FORGE_MINIO_ROOT_PASSWORD=%s\n' "$FORGE_MINIO_ROOT_PASSWORD"
    printf 'FORGE_DATABASE_URL=postgresql://%s:%s@%s:%s/%s\n' \
        "$FORGE_POSTGRES_USER" "$NEW_POSTGRES_PASSWORD" "$FORGE_POSTGRES_HOST" \
        "$FORGE_POSTGRES_PORT" "$FORGE_POSTGRES_DATABASE"
    printf 'FORGE_NATS_URL=nats://%s@%s:%s\n' \
        "$NEW_NATS_AUTH_TOKEN" "$FORGE_NATS_HOST" "$FORGE_NATS_PORT"
    printf 'FORGE_REDIS_URL=redis://:%s@%s:%s/0\n' \
        "$FORGE_REDIS_PASSWORD" "$FORGE_REDIS_HOST" "$FORGE_REDIS_PORT"
    printf 'FORGE_MINIO_ENDPOINT=%s\n' "$FORGE_MINIO_ENDPOINT"
} >"$SECRETS_TMP_FILE"
chmod 600 "$SECRETS_TMP_FILE"
mv -f "$SECRETS_TMP_FILE" "$FORGE_DEV_SECRETS_FILE"

# Recreate data-service processes without removing named volumes. PostgreSQL
# retains its role data; NATS must restart to consume the replacement token.
compose up --detach --force-recreate postgres nats
wait_for_postgres
wait_for_nats
printf 'Forge development credentials rotated; named volumes were preserved.\n'
