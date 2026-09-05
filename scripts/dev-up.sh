#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

if [[ ! -f "$FORGE_DEV_CONFIG_FILE" || ! -f "$FORGE_DEV_SECRETS_FILE" ]]; then
    "$SCRIPT_DIR/dev-init.sh"
fi

compose up --detach
wait_for_postgres
wait_for_nats
wait_for_redis
wait_for_minio
printf 'Forge development services are ready.\n'
