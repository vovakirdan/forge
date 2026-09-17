set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
set positional-arguments := true

default:
    @just --list

# Install the UI demo with its committed lockfile; no Forge services or keys needed.
ui-install:
    cd frontend && bun install --frozen-lockfile

# Open the mock-only UI on 127.0.0.1:5173; fail if that port is occupied.
ui-dev:
    cd frontend && bun run dev

# Build the imported UI demo; production hosting is not configured by this recipe.
ui-build:
    cd frontend && bun run build

# Build the static-only browser artifact; dist/server is build-time prerender output.
ui-build-static:
    cd frontend && bun run build:static

# Serve dist/client only and reuse the browser smoke plus static boundary checks.
ui-test-static:
    cd frontend && bun run test:static

# Build the isolated live entry, not the mock demo or its SSR bundle.
ui-build-live:
    cd frontend && bun run build:live

# Keyless owner login and Project read through the real Rust gateway and Core.
ui-test-live:
    @bash ./scripts/test-ui-live.sh

# Check the imported UI's TypeScript baseline without emitting files.
ui-typecheck:
    cd frontend && bun run typecheck

# Check the imported UI's lint baseline without rewriting files.
ui-lint:
    cd frontend && bun run lint

# Test isolated Core read contracts without a server, browser or provider Run.
ui-test-contracts:
    cd frontend && bun run test:contracts

# Test pure presentation of validated Core reads without a browser or services.
ui-test-presentation:
    cd frontend && bun run test:presentation

# Install Playwright's pinned Chromium; does not install Linux system packages.
ui-browser-install:
    cd frontend && bun run test:browser:install

# Test the mock-only UI with one Chromium worker and an owned local Vite server.
ui-test-browser:
    cd frontend && bun run test:browser

# Create owner-only synthetic configuration for the local development topology.
dev-init:
    @./scripts/dev-init.sh

# Start local PostgreSQL, NATS JetStream, Redis, and MinIO development services.
dev-up:
    @./scripts/dev-up.sh

# Stop development containers while preserving their named volumes.
dev-down:
    @./scripts/dev-down.sh

# Rotate local PostgreSQL/NATS credentials without removing named volumes.
dev-rotate-secrets:
    @./scripts/dev-rotate-secrets.sh

# Show the state of the local development containers.
dev-status:
    @./scripts/dev-status.sh

# Apply current Forge migrations through the forge-storage migrator binary.
migrate:
    @./scripts/migrate.sh

# Run formatting and lint gates without modifying source files.
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Run pure Rust tests without starting local data services.
test-unit:
    cargo test --workspace --all-features --locked --lib
    cargo test --package forge-testkit --all-features --locked --test command_conformance

# Local service, storage and synthetic-runtime integration; requires build-runtime-fixture.
test-integration:
    @./scripts/test-integration.sh

# The same application command engine on a services-free reference transaction.
test-command-conformance:
    cargo test --package forge-testkit --all-features --locked --test command_conformance

# Explicit actual CLI/API fixture gates; requires FORGE_PROVIDER_RUNTIME_IMAGE digest.
test-provider-integration:
    @bash ./scripts/test-provider-integration.sh

# REAL subscription usage; explicit opt-in/auth/model/image required, never automated.
test-codex-live:
    @bash ./scripts/test-codex-live.sh

# Interactive real Codex task with existing auth, private session and managed stop.
run-m1 *args:
    @bash ./scripts/run-m1.sh "$@"

# Explicit operator Git workflow in a separate empty playground; run requires approval.
run-m2 *args:
    @bash ./scripts/run-m2.sh "$@"

# M3 isolated memory acceptance; run mode requires explicit provider approval.
run-m3 *args:
    @bash ./scripts/run-m3.sh "$@"

# M3 keyless canonical memory, ownership, lifecycle, context and Gateway contracts.
test-m3:
    @bash ./scripts/run-m3.sh keyless

# Create a private toy bare Git repository; no credentials, providers or Core mutation.
prepare-m2-repository:
    @bash ./scripts/prepare-m2-repository.sh

# M2 development acceptance with real toy Git and synthetic provider observations.
test-m2:
    @bash ./scripts/test-m2.sh

# REAL paid/subscription M2 lane; explicit settings and opt-in required.
test-m2-live:
    @bash ./scripts/test-m2-live.sh

# Execute the deterministic M0 CLI operational smoke scenario.
demo-m0:
    @./scripts/demo-m0.sh

# Build pinned real CLIs and verify their versions without login/inference.
build-runtime:
    @./scripts/build-runtime-image.sh

# Synthetic executable in a real sandbox; never evidence of provider support.
build-runtime-fixture:
    cargo build --locked --package forge-supervisor --bin forge-runner
    podman build --file infra/runtime/Containerfile.fixture --tag localhost/forge-runner-fixture:m1 .
