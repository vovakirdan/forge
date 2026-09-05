#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

require_dev_environment

readonly MIGRATIONS_DIR="$FORGE_ROOT_DIR/crates/forge-storage/migrations"
if [[ ! -d "$MIGRATIONS_DIR" ]]; then
    fail "migrations are not installed yet; TASK-06 must provide forge-storage migrations and forge-migrate"
fi

exec env FORGE_DATABASE_URL="$FORGE_DATABASE_URL" \
    cargo run --quiet --locked --package forge-storage --bin forge-migrate --
