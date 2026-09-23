#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"
require_dev_environment
wait_for_postgres
wait_for_nats
export FORGE_INTEGRATION=1
exec "$FORGE_ROOT_DIR/target/debug/examples/ui_perf_fixture"
