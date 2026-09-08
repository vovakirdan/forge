# M2: shared local Run admission

One canonical database currently serves one local Supervisor. Its persisted
admission policy defaults to 16 concurrent Runs for the host, 8 per Project and
4 per credential/account. Employee capacity remains a separate limit (default 1).
These are concurrent Run counts, not aggregate money, RPM, TPM or memory promises.
Per-Run budgets and container resource limits still apply independently.

TaskStage, Communication, Resolution and provider-free Hook executions all count
against host and Project capacity. Hook does not consume Employee or account
capacity. A Human ResolutionAssignment has no Run and consumes no execution slot.

A Run with an active owned Lease **or** an unreleased physical reservation counts
once. Logical completion, cancellation, expired credentials, provider failure and
Lease revocation do not release uncertain physical ownership. Positive Supervisor
quiescence does. A temporarily full cap leaves work queued; it is not a Task failure.

Credential/account counting reads the immutable execution profile pinned in each
RunSpec. The shared group includes Runs with the same secret identity **or** the
same explicit `provider_id` + `account_id`, even across Projects with different
secret records. Missing account identity falls back to the secret identity.
Forge does not inspect secret bytes to discover undeclared account aliases: the
operator must use the same account identifier for the same upstream account.

Admissions hold the singleton policy row lock through Run creation and commit.
Counting uses a fresh database statement after lock acquisition, so simultaneous
Project transactions cannot each spend the last slot. All four issuance paths
repeat the check; selection excludes unavailable account candidates before any
provider credential opening or external work.

## Operator configuration

Core reads these non-secret environment values at startup:

| Variable | Default | Meaning |
| --- | --- | --- |
| `FORGE_HOST_MAX_RUNS` | `16` | All executions in the local database |
| `FORGE_PROJECT_MAX_RUNS` | `8` | Independent ceiling for each Project |
| `FORGE_ACCOUNT_MAX_RUNS` | `4` | Shared credential/account ceiling |

Each value must be an integer from 1 through 65535. Zero does not mean unlimited.
There are no per-Project overrides or remote-host allocation in M2.

Startup compares these values with the persisted policy under the admission lock.
Identical values are accepted during recovery. Different values are accepted only
when no Run has active or physically uncertain ownership; otherwise startup fails
closed. To change limits, stop work, confirm physical quiescence, stop Core, then
restart with the intended values. Keep the same configuration on subsequent
restarts; omitting a previously changed value requests its default again.

Project commands cannot change host-wide policy. Library composition/tests use
the explicit async `CoreService::with_admission_limits` startup builder, which
enforces the same database checks. Existing admissions always read the canonical
policy; there is no per-process cache with divergent limits.

Verification uses real PostgreSQL/Core/UDS and synthetic credentials, with manually
controlled Supervisor observations. It does not require provider inference.
