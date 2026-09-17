#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPO_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)"
cd "$REPO_DIR"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

# Builds, sandbox gate and browser acceptance run sequentially. The caller can
# wrap this entire command in a memory-limited systemd user scope.
just ui-build-live
cargo build --locked -p forge-ui -p forge-cli
cargo build --locked -p forge-testkit --example ui_core_fixture
podman image exists localhost/forge-runner-fixture:m1 || {
    printf 'Build the keyless sandbox image first: just build-runtime-fixture\n' >&2
    exit 1
}
cargo test --locked -p forge-core --lib egress::tests -- --test-threads=1
cargo test --locked -p forge-supervisor --lib \
    actual_run_sandbox_cannot_reach_owner_ui_or_control_material -- --ignored --test-threads=1
cd frontend
node scripts/run-live-tests.ts
