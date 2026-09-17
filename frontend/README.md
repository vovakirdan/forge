# Forge Control Room — local demo

This is the imported React/TanStack UI with in-memory mock services. It does not
connect to Forge Core. Tasks, employees, metrics and management actions shown in
the demo are not evidence of real execution. No Lovable account, API keys,
database, Podman services or provider Runs are needed.

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

## Start from the Forge repository root

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
Forge production host. Use the dev command for this demo; hosting and browser
authentication are separate work in [UI0.2](../docs/epics/ui0-e2-browser-api.md).

## Resource-limited checks

The shortcuts do not enforce resource limits themselves. For automated runs on
a Linux host with a user systemd manager, use this wrapper from the repository
root, replacing only the final recipe:

```sh
systemd-run --user --scope --quiet \
  -p MemoryMax=6G -p MemorySwapMax=1G -p CPUQuota=200% \
  timeout 300s just ui-build
```

Apply it separately to install, browser install/tests, contract tests, typecheck and lint. Keep checks sequential and
do not run Rust validation alongside them. A memory-limit failure remains a
failed check; investigate it rather than silently dropping the limits. For a
bounded demo session use the same wrapper with `timeout 1200s just ui-dev`.

## Current verification and remaining work

The installation/build/static-check results and browser evidence are recorded in
[FRONTEND-001](../tasks/frontend/frontend-001-local-demo-baseline.md).
The imported UI has existing fast-refresh lint warnings. They remain visible;
do not disable the rule to make the baseline look clean.

The automated demo browser smoke is described below and in
[FRONTEND-004](../tasks/frontend/frontend-004-browser-smoke.md). Component-unit
tests, CI, visual baselines and backend unavailable/offline/permission states
remain in [UI0.3](../docs/epics/ui0-e3-frontend-tooling.md). API integration,
domain alignment and real data belong to the [UI roadmap](../docs/UI_IMPLEMENTATION_PLAN.md).

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

The current screens still use `src/data/types.ts` and mock services; they do not
consume these contracts yet. Passing contract tests does not prove live API
integration. See [FRONTEND-002](../tasks/frontend/frontend-002-task-pipeline-contracts.md),
[FRONTEND-003](../tasks/frontend/frontend-003-run-read-contracts.md) and the
[field/gap map](../docs/UI_BACKEND_ALIGNMENT.md#8-read-contracts-frontend-002).

Run contracts distinguish all five assignment purposes and requested versus
observed state. Diagnostics validate the outer envelope; nested reports remain
opaque JSON, not proof of accepted work or permission to open files/URLs. Missing
measurements remain unknown. Employee/Surface reads and detailed evidence
contracts are tracked as [separate gaps](../docs/UI_BACKEND_ALIGNMENT.md#93-named-gaps-и-openapi-drift).
