#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

require_command cargo
require_command podman
require_dev_environment
wait_for_postgres
wait_for_nats
wait_for_redis
wait_for_minio
"$SCRIPT_DIR/migrate.sh"
"$SCRIPT_DIR/test-minio-bucket.sh"

env \
    FORGE_INTEGRATION=1 \
    FORGE_DATABASE_URL="$FORGE_DATABASE_URL" \
    FORGE_NATS_URL="$FORGE_NATS_URL" \
    cargo test --workspace --all-features --locked -- --ignored --test-threads=1

"$SCRIPT_DIR/demo-m0.sh"
