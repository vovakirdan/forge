#!/usr/bin/env bash
# Forge's own acceptance suite; never discovers or runs tests in an owner's repo.
set -euo pipefail
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/lib/dev.sh"
cd "$FORGE_ROOT_DIR"
require_command cargo
require_command git
require_command jq
require_dev_environment
wait_for_postgres
wait_for_nats

printf 'M2 keyless gates: real isolated Git, canonical PostgreSQL, synthetic Supervisor/provider observations.\n'
printf 'No credential discovery, paid inference, owner repository or implicit project test hook.\n'
bash "$SCRIPT_DIR/tests/m2-repository-test.sh"
bash "$SCRIPT_DIR/tests/m0-demo-test.sh"
cargo test -p forge-cli --lib --locked
# Offline settings/cleanup and scenario-size guards; all live/PG cases ignored.
cargo test -p forge-testkit --test m2_provider_live --test m2_git_proposal --locked
cargo test --locked -p forge-provider-codex -p forge-provider-claude \
    -p forge-provider-opencode -p forge-provider-common -p forge-provider-litellm --tests
cargo test -p forge-supervisor --lib --locked git
FORGE_INTEGRATION=1 cargo test -p forge-testkit --locked \
    --test command_conformance --test inbox_acceptance --test m2_claude_lane \
    --test m2_communication_runs --test m2_dispatch_constraints \
    --test m2_git_proposal --test m2_provider_profiles \
    --test m2_resolution_runs --test m2_escalation_gateway --test m2_hook_registry --test m2_hook_runs \
    -- --ignored --test-threads=1
printf 'Selected keyless M2 gates passed. This is not four-provider live acceptance or full M0-M1 regression proof.\n'
