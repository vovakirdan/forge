# Explicit Codex subscription smoke

Status: **PASS for the recorded profile on 2026-09-06**. The operator-approved
test completed with `1 passed; 0 failed` in 28.61 seconds using Codex `0.153.2`,
model `gpt-6-astra`, and image
`localhost/forge-runtime@sha256:b9f92552224c7f7d98cd725c4711c36b1703cc94260423406f8a4a9d17d2da83`.
Both Runs executed their marker commands concurrently in distinct surfaces and
homes. Named stop collected interrupted evidence; both containers exited with
code 0. Source auth was unchanged and supplied tokens were absent from checked
diagnostics. Private local evidence:
`/tmp/fc-01a076c6-52a4-7e71-9018-77268b87a0b4` (temporary, not a public archive).

This proves the tested profile and scenario, not every model, account or refresh
race. Compiling the test or passing synthetic integration alone proves none of
the live subscription behavior. Repeating the gate still needs explicit approval.

This manual gate uses the actual pinned Codex CLI in two real Forge Runs. It can
consume subscription quota or credits. Run it only after explicitly approving
that use of the selected account. Never add it to unattended CI, common
`test-integration`, or the synthetic `test-provider-integration` command.

## Prerequisites and settings

Use Linux x86_64, rootless Podman, the local Forge PostgreSQL/NATS services, and
the current runtime image built by `just build-runtime`. The image must contain
the current runner and Codex `0.153.2`, not the synthetic runtime fixture.

The operator must supply all four settings. None has a default:

| Setting | Meaning and security effect |
| --- | --- |
| `FORGE_CODEX_LIVE=1` | Explicit approval to execute real subscription requests; checked before configuration or auth access. |
| `FORGE_CODEX_AUTH_FILE` | Absolute path to an explicitly prepared owner-only regular auth snapshot; no symlinks or automatic home-directory discovery. |
| `FORGE_CODEX_MODEL` | Exact operator-chosen model available to the selected account; no inferred model or fallback. |
| `FORGE_CODEX_RUNTIME_IMAGE` | Cached immutable `image@sha256:digest` printed by the current runtime build; no implicit pull or tag substitution. |

Prepare the auth snapshot yourself outside the repository and restrict it to the
current user (`0600`, with private parent directories). Never pass token contents
as environment variables, command arguments, prompts or chat messages. The script
does not run login, inspect the host Codex home, or search for credentials.

The auth source is read but never overwritten. Both Runs initially receive the
same enrolled snapshot, then use separate writable homes. Copying auth does not
create an independent account or independent refresh token: concurrent refresh
can produce provider-side conflicts. Forge retains encrypted refresh/conflict
records; this gate does not silently copy refreshed auth back to the supplied
source. Account recovery may still require an explicit operator action.

```sh
just dev-up
just build-runtime

FORGE_CODEX_LIVE=1 \
FORGE_CODEX_AUTH_FILE='/absolute/private/exported-codex-auth.json' \
FORGE_CODEX_MODEL='<explicitly selected model>' \
FORGE_CODEX_RUNTIME_IMAGE='<current runtime image@sha256:digest>' \
  just test-codex-live
```

The command verifies offline image versions, waits for PostgreSQL/NATS, applies
Forge migrations, then runs only
`live_codex_subscription_two_parallel_runs_isolated_stop_and_evidence`.
Run it alone, without another integration process or concurrent use of this
exported auth snapshot. Do not enable verbose provider logging.

## What a passing gate means

The test creates two employees sharing one enrolled credential binding, two
Tasks, distinct surfaces and distinct runtime homes. It asks each real Codex
session to write a harmless marker in its own workspace and sleep while the
operator-side test stops execution. It verifies that both containers overlap,
their home directories have different inodes, and their initial grants match
the explicitly supplied snapshot without printing it.

The test issues `StopProjectExecution` even after a provider failure or timeout,
waits for physical quiescence, and collects runtime reports and interrupted
handoffs. Neither Task becomes successful merely because a process exits. Usage
and provider errors remain reported evidence; interrupted usage can be unknown.
The gate checks that diagnostic projections do not contain the supplied auth
tokens and that the source snapshot is unchanged.

Each Run has a 180-second managed wall limit and a 5-second stop grace. These are
execution bounds, not a guaranteed monetary or token cap. A marker timeout,
provider/auth error before both marker commands, or emergency force stop fails
the gate rather than being counted as provider support. Errors reported during
the requested interruption remain diagnostic evidence, not a claim of an
error-free completed provider turn. Model behavior may prevent the marker from being
written; no marker or inference result is fabricated.

## Retained state and interruption

The test prints only its private evidence root and exact container names, never
auth contents. Its `0700` root retains the private Secret Store, working surfaces
and technical evidence; encrypted credential and refresh/conflict records remain
in the test database. Normal post-quiescence cleanup removes ephemeral auth
copies only when writeback/redaction safety permits. Inspect and protect this
directory as credential-bearing state, including after a failed test.

Do not interrupt cleanup. If the harness is killed or the host fails, do not
assume the real containers stopped. Inspect the **exact names printed by this
test** and, if necessary, stop those containers explicitly:

```sh
podman inspect --format '{{.State.Running}}' '<exact container name printed by this test>'
podman kill --signal KILL '<exact container name printed by this test>'
```

The harness itself uses the same exact-container force-stop fallback when normal
stop cannot be confirmed; this is a failed gate, not a successful controlled-stop
proof. It cancels and joins its local Supervisor owner before final inventory,
then independently matches the unique Supervisor identity and exact private
runtime mount. A PostgreSQL failure cannot bypass that local cleanup pass.
The managed wall limit depends on the observer; it is not an independent Podman
lifetime guarantee after a harness crash. Do not delete auth, evidence, work surfaces or database records to hide a
failure or before establishing quiescence. Review retained diagnostics and use
explicit recovery actions instead of automatically retrying real requests.

For implementation-only verification, without auth or services:

```sh
cargo test -p forge-testkit --test m1_codex_live --locked --no-run
cargo test -p forge-testkit --test m1_codex_live --locked live_settings_never_default_to_host_auth_or_model
```

For each operator-approved live run, record the actual image digest, selected
model, safe test output and result. Keep failures and untested profiles distinct
from the successful profile recorded above.
