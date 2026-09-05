set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

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

# Check all local services, migrations, a disposable MinIO bucket, and ignored tests.
test-integration:
    @./scripts/test-integration.sh

# Execute the deterministic M0 CLI operational smoke scenario.
demo-m0:
    @./scripts/demo-m0.sh
