#!/usr/bin/env bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

readonly MINIO_MC_IMAGE="docker.io/minio/mc@sha256:a7fe349ef4bd8521fb8497f55c6042871b2ae640607cf99d9bede5e9bdf11727"

random_hex() {
    od -An -N12 -tx1 /dev/urandom | tr -d ' \n'
}

require_command podman
require_dev_environment
wait_for_minio

readonly MINIO_CONTAINER_ID="$(service_container_id minio)"
readonly TEMP_ENV_FILE="$(mktemp "$FORGE_DEV_DIR/.minio-test-env.XXXXXX")"
readonly BUCKET_NAME="${FORGE_MINIO_TEST_BUCKET_PREFIX}-$(random_hex)"
bucket_created=false

cleanup() {
    local status=$?

    if [[ "$bucket_created" == true ]]; then
        podman run --rm \
            --network "container:${MINIO_CONTAINER_ID}" \
            --env-file "$TEMP_ENV_FILE" \
            "$MINIO_MC_IMAGE" rb --force "forge/${BUCKET_NAME}" >/dev/null 2>&1 || true
    fi
    rm -f "$TEMP_ENV_FILE"
    exit "$status"
}
trap cleanup EXIT

printf 'MC_HOST_forge=http://%s:%s@127.0.0.1:9000\n' \
    "$FORGE_MINIO_ROOT_USER" "$FORGE_MINIO_ROOT_PASSWORD" >"$TEMP_ENV_FILE"
chmod 600 "$TEMP_ENV_FILE"

podman run --rm \
    --network "container:${MINIO_CONTAINER_ID}" \
    --env-file "$TEMP_ENV_FILE" \
    "$MINIO_MC_IMAGE" mb --ignore-existing "forge/${BUCKET_NAME}" >/dev/null
bucket_created=true

podman run --rm \
    --network "container:${MINIO_CONTAINER_ID}" \
    --env-file "$TEMP_ENV_FILE" \
    "$MINIO_MC_IMAGE" ls "forge/${BUCKET_NAME}" >/dev/null

podman run --rm \
    --network "container:${MINIO_CONTAINER_ID}" \
    --env-file "$TEMP_ENV_FILE" \
    "$MINIO_MC_IMAGE" rb --force "forge/${BUCKET_NAME}" >/dev/null
bucket_created=false

printf 'Forge MinIO disposable bucket check passed.\n'
