# Forge

Forge is a local-first control plane for durable AI engineering work. M0 provides
the deterministic Core simulator. M1 adds isolated rootless Podman execution,
scoped tools, encrypted credentials, recovery, and Codex/OpenCode provider lanes.
See the [M1 operator guide](docs/M1_RUNTIME.md) for setup and verification boundaries.

To try one real Codex task interactively on the local development stack:

```sh
just run-m1
```

The launcher asks for the task, suggests the current Codex model, and requests
approval to use the selected ChatGPT auth file and subscription quota. It builds
the runtime, starts an isolated session, and stops its work on exit. The source
auth is never changed. This is a developer launcher, not the installer or the
two-Run subscription acceptance test; see the [quick-start details](docs/M1_RUNTIME.md#быстрый-ручной-запуск).

## Local development

Forge currently targets a Linux host with rootless Podman, `podman-compose`,
Rust 1.98.0, `just`, and `protoc` (the Protocol Buffers compiler) installed.
`forge-protocol` generates the Supervisor bindings during Cargo builds, so
`protoc --version` must succeed on `PATH`. The development topology deliberately
starts PostgreSQL, NATS JetStream, Redis, and MinIO; it is not a production
installer.

```bash
just dev-init
just dev-up
just dev-status
just check
just test-unit
just build-runtime-fixture
just test-integration
```

`just dev-init` generates separate owner-only `var/dev/config.env` and
`var/dev/secrets.env` files. The latter contains random synthetic database and
NATS, Redis, and MinIO credentials; neither file is committed, overwritten, or
printed. `just dev-down` stops containers but preserves the named volumes;
removing data is intentionally a separate, explicit operation.

`just test-integration` reads those owner-only files only in its child process,
checks readiness for all four services, applies idempotent migrations, and
creates, lists, then removes a disposable MinIO bucket. It passes the local
database and NATS endpoints to the ignored acceptance suite without printing
credentials.

Each Core acceptance fixture uses its own `forge_test_<UUID>` PostgreSQL schema
without a `public` fallback. Test schemas and runtime evidence are retained for
diagnosis; tests do not clear existing shared development data.

If a synthetic local credential is exposed during development, use `just
dev-rotate-secrets`. It rotates the PostgreSQL and NATS credentials and
recreates only those service processes. Redis and MinIO credentials, named
volumes, and canonical data remain intact.

`just migrate` applies the current canonical schema through the
`forge-storage` `forge-migrate` binary. `just demo-m0` is a local operational
smoke scenario. `just test-integration` covers capacity, retry, dependency,
fences, stop, outbox, Gateway, recovery and real Podman fixture behavior. It never
imports personal provider credentials. Build the synthetic runtime fixture first
with `just build-runtime-fixture`.
Actual pinned CLI/API transport gates use the separate
`just test-provider-integration` target and its explicit fixture prerequisites;
see [the provider integration guide](infra/dev/litellm/README.md).

The Core and Supervisor are native host processes. Containers in this topology
are local data dependencies only and bind their ports to `127.0.0.1`.
PostgreSQL is sufficient for Core to expose its local command plane; if NATS is
temporarily unavailable, committed outbox Events remain durable and its
publisher reconnects in the background.

## Local control socket

The management HTTP API has no TCP listener. Read-only health/metrics exporters
use loopback ports 9878 (Core) and 9879 (Supervisor). Core and CLI resolve `api.sock` without using
the current working directory, in this order:

1. `$XDG_RUNTIME_DIR/forge/api.sock`, when `XDG_RUNTIME_DIR` is absolute;
2. `$XDG_STATE_HOME/forge/run/api.sock`, when `XDG_STATE_HOME` is absolute;
3. `$HOME/.local/state/forge/run/api.sock`, when `HOME` is absolute;
4. `/tmp/forge/api.sock`.

Unset or relative environment directories are skipped. The local socket is
owner-only; multi-user authentication remains outside M0.

M0 stores Artifact bodies as inline JSON only. The serialized UTF-8 JSON body,
including JSON syntax, is capped at 256 KiB (262144 bytes) across both the
local HTTP API and Supervisor protocol. Larger or binary evidence is deferred
to the later object-reference storage mode.
