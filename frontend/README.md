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
just ui-typecheck
just ui-lint
just ui-build
```

These map to `bun run typecheck`, `bun run lint`, and `bun run build` inside
`frontend/`. Typecheck and lint do not rewrite source files. Vite can regenerate
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

Apply it separately to install, typecheck and lint. Keep checks sequential and
do not run Rust validation alongside them. A memory-limit failure remains a
failed check; investigate it rather than silently dropping the limits. For a
bounded demo session use the same wrapper with `timeout 1200s just ui-dev`.

## Current verification and remaining work

The installation/build/static-check results and browser evidence are recorded in
[FRONTEND-001](../tasks/frontend/frontend-001-local-demo-baseline.md).
The imported UI has existing fast-refresh lint warnings. They remain visible;
do not disable the rule to make the baseline look clean.

There is no component/browser test suite wired into this package yet. The manual
browser smoke is not a substitute for that harness, which remains in
[UI0.3](../docs/epics/ui0-e3-frontend-tooling.md). API integration, domain alignment
and real data belong to the [UI roadmap](../docs/UI_IMPLEMENTATION_PLAN.md).
