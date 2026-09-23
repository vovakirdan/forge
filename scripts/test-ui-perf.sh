#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPO_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)"
cd "$REPO_DIR"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
cargo build --locked -p forge-ui -p forge-cli
cargo build --locked -p forge-testkit --example ui_perf_fixture
cd frontend
bun run build:live
bunx tsc --noEmit --project tests/perf/tsconfig.json
bunx playwright test --config playwright.perf.config.ts
