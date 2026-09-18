# Forge

The imported `frontend/` Control Room screens remain a mock prototype. A separate
live entry supports local owner login, a real Core Project, Task
list/detail with pinned Pipeline stages, Project-wide Run reads with separate
requested/observed states, and read-only Pipeline versions with a stage inspector.
Draft Task titles/descriptions can be edited through Core with revision checks
and idempotent retry; see [draft editing](tasks/frontend/frontend-011-draft-task-edit.md).
Task priority names come from a separate Project scheme read, with raw IDs and
explicit unavailable/stale states; see [priority reads](tasks/frontend/frontend-012-project-priorities.md).
Non-terminal Task priorities can be changed through Core with revision checks;
see [priority editing](tasks/frontend/frontend-013-task-priority-edit.md).
New Tasks can be created as drafts with an exact Pipeline version and safe retry;
see [draft creation](tasks/frontend/frontend-014-create-draft-task.md). Creation
does not approve the Task or request execution.
Draft DoD can be saved or cleared, then the saved Task can be explicitly approved;
see [DoD and approval](tasks/frontend/frontend-015-draft-dod-approval.md).
Approval leaves the Project execution gate unchanged, but an open gate may start work.
Tasks can be cancelled with an explicitly selected Project reason and optional
note; see [Task cancellation](tasks/frontend/frontend-016-task-cancellation.md).
Cancellation requests graceful stop; it does not prove the executor has stopped.
Task dependency reads show both directions and allow navigation between related
Tasks; see [dependency reads](tasks/frontend/frontend-017-task-dependency-reads.md).
Condition satisfaction is separate from Task readiness; editing links is deferred.
Diagnostics show availability and loaded counts, not raw logs or bodies. See the
[live UI runbook](frontend/README.md#live-owner-ui-frontend-007).
Acceptance is tracked separately for [Run reads](tasks/frontend/frontend-009-live-run-reads.md)
and [Pipeline reads](tasks/frontend/frontend-010-live-pipeline-reads.md);
deterministic fake-runtime fixtures are not provider execution proof. Pipeline
reads do not add an editor, publication or hook execution.
The [UI roadmap](docs/UI_IMPLEMENTATION_PLAN.md) and
[frontend/backend alignment](docs/UI_BACKEND_ALIGNMENT.md) define its adaptation.
Milestones UI0–UI4 precede the M4 installer; see the [epic index](docs/epics/README.md).
Mock screens and archived build scripts are not evidence of live integration.
For the local mock-only demo, see the [frontend setup](frontend/README.md):
`just ui-install` then `just ui-dev`. This does not start backend services or Runs.

Forge is a local-first control plane for durable AI engineering work. M0 provides
the deterministic Core simulator. M1 adds isolated rootless Podman execution,
scoped tools, encrypted credentials, recovery, and Codex/OpenCode provider lanes.

M2 implements the local Git delivery loop, candidate-bound review, optional
project hooks, integration, Employee capacity, Inbox and resolver queues. Its
four explicit profiles are Codex CLI, Claude CLI, OpenRouter API and OpenAI API.
The [execution ledger](docs/2026-09-06-m2-tasks.md) records verification status;
synthetic tests and registration do not establish authenticated live behavior.

For a real provider exercise from an empty repository, use the
[M2 operator launcher and scenario](docs/M2_OPERATOR_SCENARIO.md).
For keyless acceptance, start with the [M2 walkthrough](docs/M2_WALKTHROUGH.md),
[provider profiles](docs/M2_PROVIDER_PROFILES.md),
[native input](docs/M2_NATIVE_INPUT.md),
[Git source policy](docs/M2_GIT_SOURCE_POLICY.md),
[selected-file snapshots](docs/M2_FILE_SNAPSHOTS.md),
[shared admission limits](docs/M2_ADMISSION_LIMITS.md), and
[project hooks](docs/M2_PROJECT_HOOKS.md). Run the keyless suite with
`just test-m2`; `just prepare-m2-repository` creates a separate toy Git source
without editing or registering your repository. The
[paid live gate](docs/M2_LIVE_GATE.md) requires separate explicit opt-in.
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
The multi-binary integration scripts default `CARGO_BUILD_JOBS` to `2` to bound
parallel linker memory. Set it explicitly if your host supports a different cap.
PostgreSQL has an explicit 512 MiB shared-memory mount: its container default is
too small for catalog plans after many retained test schemas. Run the aggregate
service-backed suites sequentially on one development topology. Recreating the
PostgreSQL container to apply this setting preserves its named data volume.

If a synthetic local credential is exposed during development, use `just
dev-rotate-secrets`. It rotates the PostgreSQL and NATS credentials and
recreates only those service processes. Redis and MinIO credentials, named
volumes, and canonical data remain intact.

`just migrate` applies the current canonical schema through the
`forge-storage` `forge-migrate` binary. `just demo-m0` is a local operational
smoke scenario with a fresh `forge_m0_<random>` database and private Supervisor
state, sockets and host identity. It never reuses shared queues or a personal
Supervisor journal. The database and owner-only runtime logs are retained on
success and failure; the launcher prints their identifiers, never a connection
URL. Existing Runs and admission limits are not modified to make the demo pass.
`just test-integration` covers capacity, retry, dependency,
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
