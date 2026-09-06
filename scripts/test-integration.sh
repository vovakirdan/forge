#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"

require_command cargo
require_command podman
require_dev_environment
podman image exists localhost/forge-runner-fixture:m1 \
    || fail "synthetic runtime fixture missing; run just build-runtime-fixture first"
wait_for_postgres
wait_for_nats
wait_for_redis
wait_for_minio
"$SCRIPT_DIR/migrate.sh"
"$SCRIPT_DIR/test-minio-bucket.sh" --evidence

printf 'Running common dev integration with the synthetic runtime fixture.\n'
printf 'NOT RUN here: actual CLI image probes and real LiteLLM/OpenCode/API gates; use just test-provider-integration.\n'
printf 'NOT RUN here: real Codex subscription smoke; requires separate explicit just test-codex-live approval.\n'

env \
    FORGE_INTEGRATION=1 \
    FORGE_DATABASE_URL="$FORGE_DATABASE_URL" \
    FORGE_NATS_URL="$FORGE_NATS_URL" \
    cargo test --package forge-core --package forge-storage --package forge-testkit \
        --package forge-supervisor --all-features --locked -- \
        --ignored --skip uploads_redacted_evidence_to_minio \
        --skip actual_pinned_image_probes_both_clis_without_credentials \
        --skip actual_api_run_is_scoped_accounted_and_revoked_without_task_success \
        --skip live_codex_subscription_two_parallel_runs_isolated_stop_and_evidence \
        --test-threads=1

"$SCRIPT_DIR/demo-m0.sh"
