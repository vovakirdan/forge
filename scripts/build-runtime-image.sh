#!/usr/bin/env bash
# Builds only local runtime artifacts. No provider login or inference is performed.
set -euo pipefail
umask 077
readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)"
readonly IMAGE_TAG="localhost/forge-runtime:m1"
cd "$REPO_ROOT"
if [[ $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
    printf 'M1 runtime image build currently supports Linux x86_64.\n' >&2
    exit 1
fi
if [[ -n ${CARGO_TARGET_DIR:-} && ${CARGO_TARGET_DIR} != "$REPO_ROOT/target" && ${CARGO_TARGET_DIR} != target ]]; then
    printf 'Runtime image expects workspace target/debug; unset CARGO_TARGET_DIR for this build.\n' >&2
    exit 1
fi
cargo build --locked --package forge-supervisor --bin forge-runner \
    --package forge-provider-opencode --bin forge-opencode-driver
podman build --file infra/runtime/Containerfile --tag "$IMAGE_TAG" .
bash "$SCRIPT_DIR/check-runtime-image.sh" "$IMAGE_TAG"
