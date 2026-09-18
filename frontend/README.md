# Forge Control Room — local demo and live owner UI

The imported React/TanStack demo uses in-memory mock services and does not connect
to Forge Core. Its tasks, employees, metrics and management actions are not
evidence of real execution. The separate FRONTEND-007 live entry adds owner login
and read-only Project access through the native Rust gateway. FRONTEND-008 adds
the real Task list and detail card with pinned Pipeline stage. FRONTEND-009 adds
Project-wide Run list/detail and diagnostic availability in a separate Runs
section. FRONTEND-010 adds Pipeline versions and a read-only stage inspector.
FRONTEND-011 adds title/description editing for existing draft Tasks through Core.
FRONTEND-012 resolves Task priority names through a separate Project scheme read.
FRONTEND-013 adds priority editing for non-terminal Tasks through Core.
FRONTEND-014 creates draft Tasks with an explicitly selected Pipeline version.
FRONTEND-015 edits draft DoD and separately approves saved drafts through Core.
The live entry does not load the demo shell or services; Board, Team, Pipeline
editing, other commands, SSE and body/context viewers remain future work.

The demo needs no Lovable account, API keys, database, Podman services or provider
Runs. The live screen needs a running local Core and an existing Project ID;
it does not need provider credentials. Text editing keeps the Task in draft;
priority editing preserves its lifecycle/stage and does not interrupt an existing Run.
Core's normal post-command dispatch is unchanged. Acceptance results belong
in [FRONTEND-007](../tasks/frontend/frontend-007-live-owner-gateway.md),
[FRONTEND-008](../tasks/frontend/frontend-008-live-task-reads.md),
[FRONTEND-009](../tasks/frontend/frontend-009-live-run-reads.md) and
[FRONTEND-010](../tasks/frontend/frontend-010-live-pipeline-reads.md), not in the
existence of these instructions.

## Prerequisites

Use Linux, Bun **1.3.11**, Node **24.14.0**, and `just` for the root shortcuts.
The versions are recorded in `package.json`, `.bun-version` and `.node-version`;
these files do not install or switch tools automatically. Check your PATH:

```sh
bun --version
node --version
```

Keep `bun.lock` as the only lockfile. Do not run `npm install` or regenerate the
dependency tree to work around a frozen-install failure.

## Start the demo from the Forge repository root

```sh
just ui-install
just ui-dev
```

Open **http://127.0.0.1:5173/**. Stop the server with Ctrl-C. It binds to loopback
and fails if port 5173 is occupied; it does not silently switch to another port.
Stop your previous demo instance first. Do not kill an unrelated port owner.

From `frontend/`, the equivalent commands are:

```sh
bun install --frozen-lockfile
bun run dev
```

## Check and build

Run one command at a time:

```sh
just ui-test-contracts
just ui-typecheck
just ui-lint
just ui-build
```

These map to `bun run test:contracts`, `bun run typecheck`, `bun run lint`, and
`bun run build` inside `frontend/`. Contract tests, typecheck and lint do not
rewrite source files. Vite can regenerate
the tracked route manifest when routes change; the current baseline produces
no source diff. Build output lives in ignored directories; inspect
`git status --short` after checks. The existing `format` script does rewrite
files and is not part of this workflow.

The unchanged Lovable Vite wrapper builds client assets in `.output/public` and
Nitro **cloudflare-module** output in `.output/server`, including `index.mjs`
and Wrangler configuration. This is not a standalone Node server or the chosen
Forge production host. Use the dev command for this demo. FRONTEND-006 adds the
separate static demo target below. The native gateway uses the distinct live
artifact described next; the full [UI0.2](../docs/epics/ui0-e2-browser-api.md)
epic remains open.

## Live owner UI (FRONTEND-007)

Use the [backend development prerequisites](../README.md#local-development),
including Rust 1.98.0 and `protoc`, in addition to the frontend tools above.
Start your existing Core as its normal owner and keep its management API on a
private Unix socket. These instructions attach the owner UI; they do not
start Core, create a Project, migrate a database or install Forge.

From the repository root, build one target at a time:

```sh
just ui-install
just ui-build-live
CARGO_BUILD_JOBS=2 cargo build --locked -p forge-ui -p forge-cli
```

Run the gateway in one terminal, using the default socket convention:

```sh
./target/debug/forge-ui serve --assets-dir frontend/dist-live
```

It prints `Origin: http://127.0.0.1:<port>` with an OS-selected port. Open that
exact origin, not `localhost`, another port or a remote address. In a second
interactive terminal under the same owner and runtime environment:

```sh
./target/debug/forge-cli ui login
```

The current Cargo executable is `forge-cli`, even though its help calls the
command `forge`. Login requires stdin/stdout on the owner's controlling TTY;
do not pipe, redirect, capture or journal its output, or run it through the
non-TTY check wrapper below. It prints the origin, one-time code and expiry to
the terminal. Paste the code into the live screen, then enter an existing UUIDv7
Project ID. The card reads real `id`, `name`, `revision` and `execution_gate`;
health alone does not prove Project access.

The Project opens the **Tasks** section, which reads 20 Tasks at a time. Use
**Previous page**, **Next page** and **Refresh tasks**; **Open TASK-…** opens its detail.
The card shows description, Definition of Done, properties, waits, and each
artifact's ID, kind, title and creation date. Stage names come from the version
pinned by the fresh Task detail, including an old version in a soft-deleted
Pipeline catalog. List rows show raw stage/priority IDs. Priority names come from
the Project's canonical scheme, not a fixed browser enum; assignee and Run state
are not inferred from Task lifecycle.
The canonical detail response still contains inline artifact bodies and metadata;
the card does not render them or follow referenced URLs/object refs.

**Refresh priorities** reloads the shared catalog for the list and detail.
Retired levels remain visible with a marker. If a name cannot be resolved, the
raw ID remains visible; the default level is never substituted. Failed refresh
keeps previous names marked stale and does not block Task reads or draft editing.
This is a read-only catalog: Core currently creates a standard three-level scheme
and has no configuration command. One/ten-level and retired-reference browser
fixtures are explicitly synthetic. See
[FRONTEND-012](../tasks/frontend/frontend-012-project-priorities.md) for evidence.

For a draft, choose **Edit draft** to load fresh Project/Task revisions, change
**Draft title** or **Draft description**, then **Save**. Only these two fields
are editable. Whitespace and formatting are preserved; empty descriptions are
allowed. A confirmed receipt displays **Saved.**; subsequent read failure does
not undo that confirmation. **Refresh saved data** retries reads, not the command.

After a conflict, **Refresh edit baseline** shows current saved values while
retaining the fields you changed; saving again is a separate explicit action.
If the result is unknown, **Retry same save** repeats the original request/key
to obtain its receipt without applying the edit twice. Do not assume failure
means nothing was saved. Local edits and the retry key are held only in memory:
leaving warns before discarding them, and logout/closing the form does not undo
a Core commit. After a reload, read the Task again. See
[FRONTEND-011](../tasks/frontend/frontend-011-draft-task-edit.md) for acceptance.

For a non-terminal Task, **Change priority** opens a fresh Task/catalog baseline.
Choose an active level and **Save priority**; unchanged values send no command.
Only one editor is open at a time. Unknown/retired current IDs remain visible
but cannot be assigned; no default is substituted. A failed catalog read blocks
the editor's save, not the Task card.

After a conflict, **Refresh priority baseline** keeps your selected ID. If the
level is no longer active, choose another before saving explicitly. Unknown
delivery uses **Retry same save** with the original bytes/key; confirmed saves
use **Refresh saved data** for read failures. Leaving warns before discarding
local intent; logout does not undo a Core commit.
Core may update pending work and dispatch eligible work in an open Project.
Acceptance uses a separate stopped Project without Employees/provider Runs;
see [FRONTEND-013](../tasks/frontend/frontend-013-task-priority-edit.md).

**Create draft** replaces the Task detail/editor with a creation form. Enter a
title, optional description/definition of done, select delivery or analysis,
an active priority and an exact Pipeline version. Versions are paginated; neither
latest nor default is selected automatically. The active priority default may be
preselected. Properties remain empty; creation does not approve the Task or
request execution. No available Pipeline means creation is unavailable.

**Refresh creation baseline** preserves all fields after a conflict; submit
again explicitly. **Retry same creation** repeats an uncertain request with its
original bytes/key. A confirmed receipt supplies the new Task ID. If follow-up
reads fail, **Refresh created data** retries reads only; a successful read opens
the new Task card. Do not create a replacement Task to recover an unknown result.
Leaving loses the in-memory attempt, so inspect the Task list before creating
again after reload. See [FRONTEND-014](../tasks/frontend/frontend-014-create-draft-task.md).

**Edit draft** also sets, replaces or clears **Draft definition of done**.
An empty field clears it; saved nonempty text has a 20,000 Unicode-character
limit. Saving keeps the Task in draft and sends only changed fields.

**Approve draft** is a separate action. Review the saved title/DoD, exact pinned
Pipeline and Project execution gate, check **I understand approval permits
execution**, then **Confirm approval**. Missing DoD offers an edit path. Core
checks the remaining readiness conditions; having DoD alone does not promise
success. Approval does not open the gate, but work may start if it is already open.

After a refusal, **Refresh approval baseline** requires a new confirmation.
**Retry same approval** repeats an unknown attempt unchanged. **Approved.**
confirms the receipt; **Refresh approved data** retries failed reads without
sending another approval. The resulting lifecycle comes from Core and may be
ready, waiting or in_progress. Leaving or logging out cannot undo a committed
approval. See [FRONTEND-015](../tasks/frontend/frontend-015-draft-dod-approval.md).

**Refresh task** retries a read. A failed refresh keeps the previous data marked
stale; an initial failure does not appear as an empty list. If a cursor becomes
invalid, use **Restart pagination**. An oversized response reports the interface
limit (64 KiB for Task/Run lists, 1 MiB for their details and Pipeline version
list/detail), without truncation or a claim that the Task is corrupt.

Choose **Runs** to read the selected Project's Runs, then **Open Run …** for a
card. **Previous page**, **Next page**, **Refresh runs**, **Refresh run** and
**Close run** operate on this read-only section. This is not Task-specific Run
history; no Task filter or history total is inferred from a loaded page.
The card keeps desired and observed states separate: a stop request is not
confirmation that execution stopped. Purpose-specific owner fields and nullable
Task/Employee IDs stay explicit; provider/model/timestamps/cost/duration/names
are unavailable in the current DTO and are not invented.

Run diagnostics show report/handoff/usage/Git-source availability, loaded
incident/evidence counts and stream completeness. `null` means absent; `{}` is
present but proves neither success nor acceptance. Inline diagnostic objects
still arrive in the bounded detail response; the card does not display their
bodies, URLs, object refs or paths, or fetch referenced content. Core's current
Run pagination loads all Project Runs before selecting a page; browser limits
do not remove that server-side limitation. Large diagnostics may exceed 1 MiB
and produce an explicit size error.

Choose **Pipeline versions** to read versions in pages of 20, not a unique-Pipeline
catalog or a total count. Selecting a version makes a fresh detail read. The
name, catalog revision, default/latest pointers and soft-deletion state are
mutable catalog metadata; only the version's definition is immutable. Default
need not be the latest version, and soft deletion does not remove its history.
The screen does not infer pinned Task usage or offer editing/publication.

The detail lists Task kinds, entry stage, stage-visit limit, stages, transitions
and artifact requirements. The inspector initially selects the entry stage and
shows the selected stage's executor, outcomes, instructions and typed workspace,
acceptance and system-action configuration. Unresolved references retain their
IDs rather than being matched by name. Instructions are plain text; unknown
fields, raw JSON and private hook runtime data are not exposed as a viewer.
System actions are described, not executed.

A `null` workspace, acceptance policy or stage-visit limit means **not configured**.
It does not promise no WorkSurface, automatic acceptance or unlimited execution.
The list contains full version definitions and has a 1 MiB response cap, like its
detail. Core currently loads all versions before pagination and reads catalog
metadata per version (N+1); this slice does not fix that server-side limitation.
A valid page can exceed the cap: the UI reports a size error, without truncation
or silently retrying with a smaller page. Empty data and failed reads remain
distinct; a failed refresh marks retained data stale.

Switching **Tasks**, **Runs** or **Pipeline versions** cancels old reads and drops
their selection/cursor history; returning starts from page one. Changing Project also resets the section
to Tasks. A page reload clears Project selection but retains a valid session.
Navigation cancels reads immediately and removes inactive query data after the
scope change commits, without recreating still-observed old queries. Logout/401
clears session data. Reads refresh only on request; there is no polling or SSE.

Both binaries resolve the runtime directory in this order, skipping unset or
relative environment values:

1. `$XDG_RUNTIME_DIR/forge`;
2. `$XDG_STATE_HOME/forge/run`;
3. `$HOME/.local/state/forge/run`;
4. `/tmp/forge`.

The Core endpoint is `api.sock`; the distinct UI control endpoint is
`ui-control.sock`. If Core uses another path, pass explicit absolute paths,
replacing the examples with your owner-private directory:

```sh
./target/debug/forge-ui serve --assets-dir frontend/dist-live \
  --core-socket /absolute/owner-private/api.sock \
  --control-socket /absolute/owner-private/ui-control.sock
```

```sh
./target/debug/forge-cli ui login \
  --control-socket /absolute/owner-private/ui-control.sock
```

The gateway can create a missing private control directory. It refuses to repair
an existing unsafe path: the immediate parent must be owned by this user with
mode `0700`, sockets must be `0600`, path components must not be symlinks, and
the connected peer UID must match. CLI `--socket` is not the UI control option.
An existing live control listener causes startup to fail; do not delete its
socket or kill an unrelated process.

Codes expire after five minutes; a new login command invalidates the previous
code, and five incorrect code attempts invalidate the pending code. Sessions
last at most eight hours and live only in gateway memory and browser
`sessionStorage`. Logout revokes the current session, including token copies in
other tabs. If the network is unavailable, the screen distinguishes local logout
from unconfirmed server revocation. Gateway restart invalidates all sessions;
it may also select a different port. Core unavailability is shown explicitly,
without mock data or automatic login retries. Stop the gateway with Ctrl-C;
its owned control socket is removed, while its lifetime lock file is retained.

`vite.live.config.ts` uses the installed React/Tailwind plugins and TanStack
Query, not TanStack Start or the Lovable wrapper. `frontend/dist-live` contains
the HTML, assets and `forge-live-manifest.json`. The gateway validates hashes
and paths, then serves an immutable in-memory snapshot: changing files requires
a rebuild and gateway restart. The manifest is an integrity allowlist for a
trusted owner build, not a publisher signature. Client env export, public-dir
copying and source maps are disabled. The live shell uses strict CSP without
inline scripts, eval or third-party scripts; keep secrets out of source and
assets. Node is needed for build/test only, not for serving this artifact.

### Live checks

Service-free session/API/manifest/config tests run from the repository root:

```sh
(cd frontend && bun run test:live-unit)
```

For the separate keyless browser acceptance, prepare the local development
services and pinned Playwright Chromium as documented above and in the backend
README. PostgreSQL and NATS must be ready; the synthetic sandbox image is also
required. From the repository root, sequentially:

```sh
just ui-browser-install
just build-runtime-fixture
just ui-test-live
```

`ui-test-live` builds the live artifact, gateway/CLI and test fixture, checks the
production sandbox boundary without a provider, then launches browser tests with
real Core and an isolated PostgreSQL schema. The fixture creates its own Projects,
Tasks, artifacts and versioned Pipelines through named commands; it does not
reuse personal queues or require inference. Task browser checks include page20,
cross-Project isolation, an empty Project, old pins after default change/soft
deletion, delayed real reads, keyboard/narrow layout and explicit recovery.
The Run fixture uses separate Projects with 23 + 1 Runs and an empty Project.
Named commands and M0 fake runtime create the records; the fixture waits for
observed Stopped before publishing its coordinates. This proves real Core reads
with deterministic test execution, not a provider or sandbox Run. Synthetic
cases for all five purposes, nullable owners, independent states, diagnostic
contents and injected failures are labelled separately from real-Core evidence.
Pipeline acceptance uses separate Projects with 23 + 1 versions and an empty
Project, created through named commands without provider Runs. It covers an
older default, a soft-deleted catalog and a full-definition list larger than
64 KiB but within 1 MiB; policy/executor variants and injected failures use
separately labelled synthetic cases. Run and Pipeline acceptance outcomes belong
in FRONTEND-009 and FRONTEND-010 respectively; this runbook does not claim their
checks passed.
The suite does not run the broad backend integration target. Its temporary
processes stop on exit; PostgreSQL test schemas are retained for diagnosis.

Live Playwright uses one worker, no retries and no trace, HAR, screenshot, video
or stored auth fixture. Ignored failure diagnostics live under
`frontend/test-results/`. A separate intentional failure checks issued code/token
values against output and artifacts; do not enable richer auth recording to debug
it. Source/tests and the existence of this recipe are not a PASS claim: see the
Task's evidence for commands actually run and remaining checks.

## Static build and smoke (FRONTEND-006)

From the repository root, after the usual frozen install and Chromium setup:

```sh
just ui-build-static
just ui-test-static
```

Run these separately and sequentially with the resource limits below. The test
command does not rebuild: rebuild static assets after source changes, and after
running the ordinary build, before testing them. It must fail if the shell is
missing; it never falls back to a dev or SSR server.

`vite.static.config.ts` uses the existing Lovable wrapper with TanStack SPA mode
and Nitro disabled. The static demo artifact is **`frontend/dist/client`**, including
`_shell.html`. TanStack still builds a server bundle to prerender the shell at
build time. Do not ship or serve that bundle: a running Node/Nitro/SSR process is
not required for this static UI. The existing dev and ordinary build commands
are unchanged; no dependency or lockfile upgrade is involved.

Automatic wrapper environment definitions and Vite client env export are off
for this target. This does not promise that build tools cannot read `.env` or
the host environment; never place secrets in client source or public assets.
There is no runtime public environment configuration or credential delivery.

The smoke starts a test-only Node built-in file server on
**http://127.0.0.1:4174/**. It serves only built client files and allowlisted UI
navigation paths, without importing the SSR bundle, proxying requests, or
connecting to Core. Missing assets, `/api`, `/v1`, `/_serverFn`, and traversal
attempts are errors, not successful SPA fallback. A busy port fails explicitly;
Playwright starts/stops its own server and never reuses or kills another one.

The suite reuses the seven demo scenarios and adds static route/reload and
boundary checks. Node tests use synthetic files to exercise file containment.
The same console/page-error collector, one Chromium worker and no retries
apply. Reports remain in ignored test output directories. Test/config/scripts
are included in typecheck. Do not run static and dev browser suites together.

This proves **mock-only UI from static assets**, not authentication or live Core
integration. FRONTEND-007 implements its separate live entry and Rust/Axum
gateway over private Core UDS; it does not serve this demo artifact. The
[ADR](../docs/UI_BROWSER_BOUNDARY.md) distinguishes both boundaries. Historical
static results remain in [FRONTEND-006 evidence](../tasks/frontend/frontend-006-static-hosting-boundary.md).
Tauri, remote control, installer, component harness and CI remain separate work.

## Resource-limited checks

The shortcuts do not enforce resource limits themselves. For automated runs on
a Linux host with a user systemd manager, use this wrapper from the repository
root, replacing only the final recipe:

```sh
systemd-run --user --scope --quiet \
  -p MemoryMax=6G -p MemorySwapMax=1G -p CPUQuota=200% \
  timeout 300s just ui-build
```

Apply it separately to install, browser install/tests, contract tests, typecheck
and lint. For the aggregate live acceptance, use the same memory/CPU limits
with `timeout 1200s just ui-test-live`; it builds Rust and frontend sequentially
and defaults Cargo jobs to two. Keep checks sequential and do not run another
Rust validation alongside them. A memory-limit failure remains a
failed check; investigate it rather than silently dropping the limits. For a
bounded demo session use the same wrapper with `timeout 1200s just ui-dev`.

## Current verification and remaining work

The installation/build/static-check results and browser evidence are recorded in
[FRONTEND-001](../tasks/frontend/frontend-001-local-demo-baseline.md).
The imported UI has existing fast-refresh lint warnings. They remain visible;
do not disable the rule to make the baseline look clean.

The automated demo browser smoke is described below and in
[FRONTEND-004](../tasks/frontend/frontend-004-browser-smoke.md). Component-unit
tests, CI, visual baselines and full-screen backend unavailable/offline/permission
coverage remain in [UI0.3](../docs/epics/ui0-e3-frontend-tooling.md). FRONTEND-007
adds these failure states only for its isolated login/Project screen. The other
API integrations and domain alignment remain in the
[UI roadmap](../docs/UI_IMPLEMENTATION_PLAN.md).

## Browser smoke (demo only)

From the repository root, run these commands in order:

```sh
just ui-install
just ui-browser-install
just ui-test-browser
```

The browser installation downloads Chromium for the pinned Playwright version;
it needs network access and disk space. Tests use that browser, never a system
Chrome fallback. Missing Linux shared libraries are an explicit setup failure:
the recipe does not run `sudo` or install OS packages. If the launch reports
missing libraries, use the [Playwright system dependency instructions](https://playwright.dev/docs/browsers#install-system-dependencies)
to provision that host separately, then rerun the smoke suite.

The suite starts its own Vite process at **http://127.0.0.1:4173/**, refuses an
occupied port and stops its own server after completion. It does not reuse or
stop a manual demo on 5173. No Core, database, containers, credentials or paid
model calls are involved. The imported in-memory services supply demo data;
these checks do not prove that real Forge commands or provider Runs work.

One Chromium worker runs isolated tests without retries: navigation/reload,
Task drawer and Full page, command palette search, keyboard/focus behaviour,
and dialogs at a narrow viewport. The narrow check does not certify the whole
desktop layout as mobile-ready. Browser console errors and unhandled page
errors fail the suite. Contract tests remain a separate command.

Failure traces/screenshots live in ignored `frontend/test-results/`; the ignored
`frontend/playwright-report/` HTML report never opens automatically. These are
diagnostics, not screenshot golden baselines. Tests/config participate in
`just ui-typecheck`. Use the resource-limited wrapper above for each command;
do not run two suites or a build alongside the browser run in this checkout.

Smoke waits for client-loaded project data before interacting with SSR markup.
An early-click hydration race was observed during test development; this
readiness gate does not claim to fix the underlying early-interaction path.
See the Task evidence for that remaining UI0.3 check and known tooling warnings.
During normal cleanup Bun may print server exit 143 for SIGTERM; the suite's
final result and exit code determine whether the run passed.

## Core read contracts

`src/contracts/` is an isolated schema/type layer for the existing Project, Task,
PipelineVersion and Run read responses. It uses the installed Zod package and
Node's built-in test runner with synthetic wire-shaped fixtures. Run
`just ui-test-contracts` from the repository root, or `bun run test:contracts`
from this directory. Tests run sequentially and need no services or keys.

Node executes these TypeScript tests by stripping types; `ui-typecheck` remains
the separate compiler check. Contract modules use relative `.ts` imports and
erasable syntax, not Vite aliases, TS enums or JSX. No additional runner or
dependency installation is needed after the normal frozen install.

The imported demo screens still use `src/data/types.ts` and mock services.
The separate live entry consumes Project/Task/PipelineVersion/Run contracts for
real reads; imported demo screens remain disconnected. FRONTEND-009 reuses the
existing Run schemas; FRONTEND-010 reuses Pipeline list/detail schemas without
changing the Core wire format. Passing contract tests alone does not prove live API
integration. See [FRONTEND-002](../tasks/frontend/frontend-002-task-pipeline-contracts.md),
[FRONTEND-003](../tasks/frontend/frontend-003-run-read-contracts.md) and the
[field/gap map](../docs/UI_BACKEND_ALIGNMENT.md#8-read-contracts-frontend-002).

Run contracts distinguish all five assignment purposes and requested versus
observed state. Diagnostics validate the outer envelope; nested reports remain
opaque JSON, not proof of accepted work or permission to open files/URLs. Missing
measurements remain unknown. Employee/Surface reads and detailed evidence
contracts are tracked as [separate gaps](../docs/UI_BACKEND_ALIGNMENT.md#93-named-gaps-и-openapi-drift).

## Task and Run presentation

`src/presentation/` derives display facts from already validated Core DTOs.
`presentTaskSummary`, `presentTaskDetail`, `presentRun` and
`presentRunDiagnostics` each return `{ source, presentation }`. `source` is the
original DTO, not a clone or a legacy demo model. Treat it as read-only; recompute
presentation when the source changes. The functions add English labels and
loaded-data counts without interpreting arbitrary JSON or performing I/O.

Task kind/lifecycle labels stay separate from stage resolution, which uses only
the pinned PipelineVersion. `no_stage` and the reasons for an `unavailable` stage
remain distinct. Priority stays a stable ID in `source`; no rank, color, catalog
or cancellation reason is invented. Summary responses have no detail-only counts.

Run purpose and requested/observed states have independent labels. The typed
owner stays in `source.assignment`; Employee IDs are not names or profiles.
Presenting several Runs does not select a current Run or change a Task lifecycle.
Diagnostics distinguish absent (`null`) from present (including `{}`) reports.
Counts describe only loaded incidents, evidence and incomplete streams. Presence
does not prove success, completeness of stored history or accepted work.

FRONTEND-010 adds pure Pipeline version/stage presentation for the read-only
inspector. Catalog metadata stays distinct from immutable definition data;
missing policies mean not configured, not an inferred execution policy. Neither
loaded versions nor stage counts imply total catalog size or pinned Task usage.

Run `just ui-test-presentation` from the repository root, or
`bun run test:presentation` here. Like contract tests, these synthetic tests use
Node's built-in runner and relative `.ts` imports without services, keys or a
browser. Run them sequentially with the same resource limits as the other checks.
The imported screens still use demo services. The separate live entry uses this
presentation layer for Task, Run and Pipeline reads. See
[FRONTEND-005](../tasks/frontend/frontend-005-task-run-presentation.md) and
[FRONTEND-009](../tasks/frontend/frontend-009-live-run-reads.md), plus
[FRONTEND-010](../tasks/frontend/frontend-010-live-pipeline-reads.md).
