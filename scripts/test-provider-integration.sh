#!/usr/bin/env bash
# Opt-in real CLI/API transport gates against synthetic, internal-only services.
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=lib/dev.sh
source "$SCRIPT_DIR/lib/dev.sh"
cd "$FORGE_ROOT_DIR"

readonly RUNTIME_IMAGE="${FORGE_PROVIDER_RUNTIME_IMAGE:-}"
if [[ ! "$RUNTIME_IMAGE" =~ ^[a-zA-Z0-9][a-zA-Z0-9._:/-]*@sha256:[a-f0-9]{64}$ ]]; then
    fail "set FORGE_PROVIDER_RUNTIME_IMAGE to the digest printed by just build-runtime; this gate never skips missing prerequisites"
fi
require_command cargo
require_command podman
require_command curl
require_rootless_podman
podman image exists "$RUNTIME_IMAGE" \
    || fail "requested runtime image is not cached; run just build-runtime and select its digest"

# Never aim synthetic master keys at an arbitrary listener or a public upstream.
for service in postgres stub litellm; do
    [[ $(podman inspect --format '{{.State.Running}}' "forge-litellm-fixture_${service}_1") == true ]] \
        || fail "start the dedicated fixture in infra/dev/litellm/README.md first"
done
[[ $(podman network inspect --format '{{.Internal}}' forge-litellm-fixture_fixture) == true ]] \
    || fail "the LiteLLM fixture network must be internal-only"
[[ $(podman port forge-litellm-fixture_litellm_1 4000) == 127.0.0.1:4007 ]] \
    || fail "the dedicated LiteLLM fixture must own loopback port 4007"

require_dev_environment
wait_for_postgres
wait_for_nats
"$SCRIPT_DIR/migrate.sh"
ready=false
for ((attempt = 0; attempt < 120; attempt++)); do
    if curl --noproxy '*' --fail --silent --max-time 2 \
        http://127.0.0.1:4007/health/liveliness >/dev/null; then
        ready=true
        break
    fi
    sleep 1
done
[[ "$ready" == true ]] || fail "LiteLLM fixture did not become healthy"

# Tests use two historical variable names; operators select one immutable image.
export FORGE_RUNTIME_PROBE_IMAGE="$RUNTIME_IMAGE"
export FORGE_OPENCODE_FIXTURE_IMAGE="$RUNTIME_IMAGE"
export FORGE_LITELLM_CONTRACT=1
export FORGE_INTEGRATION=1
printf 'Running real provider transport gates serially; no login, host auth or paid inference.\n'
printf 'Keep the shared LiteLLM fixture stable: do not restart it during these gates.\n'

cargo test --package forge-supervisor --lib --all-features --locked \
    podman::version::tests::actual_pinned_image_probes_both_clis_without_credentials \
    -- --ignored --exact --test-threads=1
cargo test --package forge-provider-litellm --test live_contract --locked -- --ignored --test-threads=1
cargo test --package forge-provider-opencode --test live_runtime --locked -- --ignored --test-threads=1
cargo test --package forge-testkit --test m1_api_lane --locked -- --ignored --test-threads=1
printf 'Real CLI image, LiteLLM, OpenCode transport and Core API fixture gates passed. Paid-provider support is not certified.\n'
