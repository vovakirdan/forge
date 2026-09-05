#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

require_command podman
require_command podman-compose
require_rootless_podman
require_dev_environment
wait_for_postgres
wait_for_nats
wait_for_redis
wait_for_minio
