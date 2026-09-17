#!/usr/bin/env bash
set -euo pipefail

# Test-only launcher. Load local service credentials inside this child process,
# not into the browser, gateway or build environment.
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"
require_dev_environment
wait_for_postgres
wait_for_nats
export FORGE_INTEGRATION=1
exec "$FORGE_ROOT_DIR/target/debug/examples/ui_core_fixture"
