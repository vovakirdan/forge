# LiteLLM API lane

Forge pins the published LiteLLM **1.99.0** distribution. The previously proposed
1.99.1 pin was corrected after its PyPI URL returned 404 and GHCR tags could not
be resolved. The [1.99.0 package](https://pypi.org/project/litellm/1.99.0/) and
[versioned management code](https://github.com/BerriAI/litellm/tree/v1.99.0/litellm/proxy/management_endpoints)
are the contract sources. The container base is digest-pinned; deployment must
record the built LiteLLM image digest too.

This adjunct owns a **separate database**. Never point it at Forge canonical
tables. Its PostgreSQL records are derived provider-routing and accounting state,
not Task or Run authority. Database migrations are LiteLLM's responsibility.

Build with `podman build -t localhost/forge-litellm:1.99.0 -f infra/dev/litellm/Containerfile infra/dev/litellm`.
Launch `compose.yaml` only after supplying the following variables through a
private local environment, not a tracked `.env` or command-line argument:

| Variable | Owner and effect | Default |
| --- | --- | --- |
| `FORGE_LITELLM_DATABASE_URL` | Operator; dedicated PostgreSQL database | Required |
| `FORGE_LITELLM_MASTER_KEY` | Operator/Core; administration only, never Run-visible | Required |
| `FORGE_LITELLM_SALT_KEY` | Operator; persistent encryption key for stored model credentials; back up with the database | Required |
| `FORGE_LITELLM_PORT` | Operator; loopback-only administration/inference port | `4000` |

Core creates an immutable route per project, credential binding/version, provider
and model through OSS `/model/new`. `STORE_MODEL_IN_DB=True` enables LiteLLM's
encrypted parameter storage. The route alias and deployment ID must both match;
duplicate aliases or changed metadata fail closed. Core does not silently edit a
route to another account. Routes are not deleted when one Run finishes because
other Runs may share them.

Core generates and seals a virtual key **before** issuance, then supplies that
same key to `/key/generate`. Readback by SHA256 identity checks Run/fence/epoch,
models, inference-only routes, expiry, rates and budget. A lost creation response
is reconciled with the persisted key. No request can mint an untracked replacement
key. Automatic rotation and budget reset are disabled. Supplied-key creation must
remain enabled in this dedicated service's settings.

The Run sees only its virtual key. Upstream API keys and the LiteLLM master key
stay outside the sandbox. Revocation uses the checked key hash, not a potentially
non-unique alias; usage import is
observed, possibly delayed spend, not final accounting or Task completion. Budget
checks can overshoot with already in-flight requests. Core remains responsible
for lease revocation, stop, incidents and lifecycle changes.

Normal logging disables message bodies and per-request spend logs. It does not
make verbose/debug modes safe: do not enable them in a credential-bearing service.
Persist the salt and database together; missing salt must never silently recreate
or rebind routes. Compose environment interpolation can print credentials: do not
publish `podman-compose config` or unredacted launch output.

Verification without upstream credentials:

```sh
cargo test -p forge-provider-litellm -p forge-provider-opencode --all-features --locked
```

The normal suite uses synthetic HTTP peers. It proves client request/readback
behavior, not actual LiteLLM enforcement or paid-provider support. The opt-in
live contract test additionally requires the dedicated local fixture service.

There are two separate integration entry points. `just test-integration` checks
the common local services and synthetic runtime (`just build-runtime-fixture`
first). It explicitly excludes real CLI image probes and the full API lane; its
success is not a provider certification. `just test-provider-integration` runs
the four opt-in real transport gates and fails when prerequisites are missing.

Build the current runtime and the LiteLLM fixture image, start the isolated
fixture, then supply the runtime digest printed by `just build-runtime`:

```sh
just dev-up
just build-runtime
podman build -t localhost/forge-litellm:1.99.0 -f infra/dev/litellm/Containerfile infra/dev/litellm
podman-compose -p forge-litellm-fixture -f infra/dev/litellm/fixture-compose.yaml up -d
FORGE_PROVIDER_RUNTIME_IMAGE='<built runtime image@sha256:digest>' just test-provider-integration
```

`FORGE_PROVIDER_RUNTIME_IMAGE` is an operator-selected, non-secret, cached
digest-pinned image reference, with no default. The script passes that same
image to the credential-free version probe and OpenCode/Core API fixtures.
No image is silently pulled, provider login initiated or host auth imported.
Local PostgreSQL and NATS configuration comes from `just dev-init`; migrations
are applied before the full API gate. The dedicated fixture must have its
internal-only network and loopback port 4007; readiness is checked explicitly.
The Core harness uses a separate retained PostgreSQL schema per fixture. Keep
the shared LiteLLM service stable during these gates; do not restart it in parallel.
Changing runner/driver code requires rebuilding and
selecting the new digest before claiming current-source verification.

The equivalent individual provider commands remain useful for diagnosis:

```sh
podman-compose -p forge-litellm-fixture -f infra/dev/litellm/fixture-compose.yaml up -d
FORGE_LITELLM_CONTRACT=1 cargo test -p forge-provider-litellm --test live_contract -- --ignored
FORGE_LITELLM_CONTRACT=1 FORGE_OPENCODE_FIXTURE_IMAGE='<built runtime image@sha256:digest>' \
  cargo test -p forge-provider-opencode --test live_runtime -- --ignored
podman-compose -p forge-litellm-fixture -f infra/dev/litellm/fixture-compose.yaml down
```

The fixture has an internal-only container network, a synthetic HTTP provider,
and a dedicated ephemeral PostgreSQL data directory. No paid API route is
reachable. It binds only `127.0.0.1:4007`; its checked-in keys are deliberately
synthetic and must never be used for a real deployment. Wait for LiteLLM health
before running the test. `down` removes only this named fixture; no Forge project
or canonical database is modified.

The OpenCode test uses the actual pinned server in a uniquely named ephemeral
container and a Run-scoped virtual key. It validates session creation, streaming
completion and normalized usage. Its fixture-only network/MCP substitutions do
not prove Core authorization, Gateway or Supervisor sandbox enforcement; those
remain separate acceptance gates.

The `forge-testkit` API acceptance gate covers that full composition instead:
real Core, its scoped Unix Gateway, rootless Supervisor sandbox, the managed
OpenCode driver and real LiteLLM. It requires the local Forge development services
and applied migrations in addition to this fixture. Run it serially against the
stable synthetic provider service:

```sh
source scripts/lib/dev.sh
require_dev_environment
FORGE_INTEGRATION=1 FORGE_LITELLM_CONTRACT=1 \
  FORGE_OPENCODE_FIXTURE_IMAGE='<built runtime image@sha256:digest>' \
  cargo test -p forge-testkit --test m1_api_lane --locked -- --ignored --test-threads=1
```

Only `fixture-compose.yaml` sets `OPENAI_API_BASE=http://stub:4101/v1`, the
[1.99.0 OpenAI route fallback](https://github.com/BerriAI/litellm/blob/v1.99.0/litellm/main.py).
This redirects Core-created immutable routes to the synthetic upstream without
changing production route policy. The internal network still prevents public
egress. The acceptance gate waits for fixture health before creating Run
authority and distinguishes physical quiescence from the later broker revocation
acknowledgement. Retained work surfaces and redacted evidence are intentional;
there is no successful Task outcome merely because OpenCode exited with code 0.

The live key contract verifies the observed 1.99.0 budget rejection envelope:
HTTP `429`, `error.type = "budget_exceeded"`, `error.code = "429"`. A generic
429 may be a rate limit; Core must not infer a budget incident from HTTP status
or arbitrary message text alone.

## Verification record — 2026-09-06

- Built image: `localhost/forge-litellm@sha256:5702ff0c9f45ea48729d0a94bbbb2a9774b7d18a98b4413d83dc71c8e7370748`.
- Published package reports `LiteLLM: Current Version = 1.99.0`; Prisma client
  generation and migrations ran successfully in the dedicated fixture database.
- Live key contract passed: exact immutable route create/reconcile, supplied-key
  reconciliation, model/admin-route denial, expiry, spend, budget rejection and
  exact-key revocation.
- Actual OpenCode `1.18.29` image
  `localhost/forge-runtime@sha256:d191a0d5dc0144b22a6282da5b9e2c204facdc57531feec7c81f47a72e8a7779`
  completed a fresh session through LiteLLM against the stub. Output and reported
  input/output usage matched the fixture. A second session was aborted through
  OpenCode's real API after its busy event; no completion was emitted. No paid
  upstream call was made.
- Full Core API acceptance passed in 20.99 seconds with rebuilt runtime
  `localhost/forge-runtime@sha256:e6e691b106baeb318634c94c2ebeb590de44f66be4df3bad3e988da647cf7a0c`.
  It checked real model/admin-route denial, scoped Gateway inference, 1000 input
  and 1000 output tokens, exit code 0, positive physical quiescence and eventual
  key revocation. The Task stayed `waiting` with an interrupted handoff, not
  `done`. Canonical RunSpec, grants and diagnostic projections contained no
  upstream or master key; RunSpec and diagnostics contained no virtual key.
- After the final runner redaction fixes, `just test-provider-integration`
  passed all four gates with runtime
  `localhost/forge-runtime@sha256:b9f92552224c7f7d98cd725c4711c36b1703cc94260423406f8a4a9d17d2da83`:
  credential-free image probes (7.86 s), LiteLLM contract (7.26 s), actual
  OpenCode completion/abort (18.13 s), and full Core API lane (20.62 s).
  The separate synthetic runtime was also rebuilt as
  `localhost/forge-runner-fixture@sha256:80ccd11f9fd63a9e338cd80392f9dcaba43d2d36c1121bbb21413f27c5400bc9`;
  that image is only for common integration, not provider certification.

The first empty database startup took approximately one minute. Readiness, not a
fixed sleep, must gate clients. These records apply to the stated digests; a new
build must rerun the gates because transitive Python dependencies are resolved
at image build time. No paid-provider credential mode is certified by this test.

When editing/restarting the stub, recreate/restart the whole fixture stack:
rootless Podman can assign a new stub IP while LiteLLM still caches the previous
connection target. This caused one bounded test timeout during development; a
fresh fixture passed both the completion and abort gates in 24.28 seconds.
