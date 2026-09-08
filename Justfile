set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
set positional-arguments := true

default:
    @just --list

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
