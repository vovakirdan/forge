import { chmod, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test as base, expect, type ConsoleMessage, type Page } from "@playwright/test";
import { z } from "zod";
import { firstLine, launch, stop, terminalLogin } from "./process";
import { registerSecrets, sanitizeErrors } from "./secrets";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const CommandProjectSchema = z.object({
  id: z.string().uuid(),
  name: z.string(),
  drafts: z.array(z.object({ id: z.string().uuid(), key: z.string() })).length(12),
  cancelled_id: z.string().uuid(),
});
const CoreFixtureSchema = z.object({
  core_socket: z.string(),
  project_id: z.string().uuid(),
  project_name: z.string(),
  revision: z.number().int().positive(),
  execution_gate: z.literal("open"),
  tasks: z.object({
    total: z.number().int().positive(),
    featured_id: z.string().uuid(),
    featured_key: z.string(),
    featured_title: z.string(),
    second_id: z.string().uuid(),
    second_key: z.string(),
    draft_id: z.string().uuid(),
    draft_key: z.string(),
    pinned_version_id: z.string().uuid(),
    default_version_id: z.string().uuid(),
    pipeline_id: z.string().uuid(),
    stage_id: z.string(),
    stage_name: z.string(),
    artifact_titles: z.array(z.string()),
  }),
  second_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    task_id: z.string().uuid(),
    task_title: z.string(),
  }),
  empty_project: z.object({ id: z.string().uuid(), name: z.string() }),
  commands_project: CommandProjectSchema,
  priority_commands_project: CommandProjectSchema,
  cancellation_commands_project: CommandProjectSchema.extend({
    ready_id: z.string().uuid(),
    waiting_id: z.string().uuid(),
  }),
  approval_commands_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    drafts: z.array(z.object({ id: z.string().uuid(), key: z.string() })).length(12),
    pipeline_version_id: z.string().uuid(),
    without_dod_id: z.string().uuid(),
    human_id: z.string().uuid(),
    external_id: z.string().uuid(),
    dependency_id: z.string().uuid(),
    blocker_id: z.string().uuid(),
    deleted_pipeline_task_id: z.string().uuid(),
  }),
  create_commands_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    pipeline_id: z.string().uuid(),
    older_version_id: z.string().uuid(),
    latest_version_id: z.string().uuid(),
    deleted_pipeline_id: z.string().uuid(),
    deleted_version_id: z.string().uuid(),
    delivery_only_version_id: z.string().uuid(),
    foreign_project_id: z.string().uuid(),
    foreign_version_id: z.string().uuid(),
  }),
  runs_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    total: z.number().int().positive(),
    featured_id: z.string().uuid(),
    second_id: z.string().uuid(),
    featured_task_id: z.string().uuid(),
    employee_id: z.string().uuid(),
    stage_id: z.string(),
  }),
  other_runs_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    run_id: z.string().uuid(),
  }),
  pipelines_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    total: z.number().int().positive(),
    featured_id: z.string().uuid(),
    second_id: z.string().uuid(),
    deleted_id: z.string().uuid(),
    pipeline_id: z.string().uuid(),
    stage_id: z.string(),
    stage_name: z.string(),
  }),
  other_pipelines_project: z.object({
    id: z.string().uuid(),
    name: z.string(),
    version_id: z.string().uuid(),
  }),
});
type CoreFixture = z.infer<typeof CoreFixtureSchema>;
export type Live = {
  origin: string;
  control: string;
  core: CoreFixture;
  secrets: Set<string>;
  login: (page: Page) => Promise<void>;
  bearer: () => Promise<string>;
  allowConsoleErrors: <T>(
    pattern: RegExp,
    action: () => Promise<T>,
    maximum?: number,
  ) => Promise<T>;
};

export const test = base.extend<{ live: Live }, { core: CoreFixture }>({
  core: [
    // Playwright requires a destructured fixture argument, even without dependencies.
    // eslint-disable-next-line no-empty-pattern
    async ({}, provide) => {
      const child = launch("bash", [join(root, "scripts/ui-core-fixture.sh")], root);
      child.stderr.resume();
      try {
        let core: CoreFixture;
        try {
          core = CoreFixtureSchema.parse(JSON.parse(await firstLine(child)));
        } catch {
          throw new Error("Real Core fixture startup failed; check PostgreSQL/NATS");
        }
        await provide(core);
      } finally {
        await stop(child);
      }
    },
    { scope: "worker" },
  ],
  live: async ({ core, page }, provide, info) => {
    let browserFaults = 0;
    const faultKinds = new Map<string, number>();
    let browserEvents = 0;
    let expectedConsole: { pattern: RegExp; remaining: number } | null = null;
    const watched = new Map<
      Page,
      { console: (message: ConsoleMessage) => void; error: () => void }
    >();
    const fault = (kind: string) => {
      browserFaults = Math.min(browserFaults + 1, 128);
      faultKinds.set(kind, Math.min((faultKinds.get(kind) ?? 0) + 1, 128));
    };
    const watch = (opened: Page) => {
      if (watched.has(opened)) return;
      if (watched.size >= 16) {
        fault("too_many_pages");
        return;
      }
      const onConsole = (message: ConsoleMessage) => {
        if (message.type() !== "error") return;
        browserEvents += 1;
        if (browserEvents > 128) {
          fault("too_many_console_events");
          return;
        }
        if (
          expectedConsole &&
          expectedConsole.remaining > 0 &&
          expectedConsole.pattern.test(message.text())
        ) {
          expectedConsole.remaining -= 1;
        } else {
          const status =
            /^Failed to load resource: the server responded with a status of ([45][0-9]{2}) /.exec(
              message.text(),
            )?.[1];
          const network =
            /^Failed to load resource: net::(ERR_ABORTED|ERR_FAILED|ERR_INTERNET_DISCONNECTED|ERR_CONNECTION_CLOSED)$/.exec(
              message.text(),
            )?.[1];
          // Only finite categories or a three-digit HTTP status cross diagnostics.
          fault(
            status ? `console_http_${status}` : network ? `console_${network}` : "console_other",
          );
        }
      };
      // Never persist browser exception text: it could contain session material.
      const onError = () => fault("page_error");
      opened.on("pageerror", onError);
      opened.on("console", onConsole);
      watched.set(opened, { console: onConsole, error: onError });
    };
    page.context().on("page", watch);
    for (const opened of page.context().pages()) watch(opened);
    async function allowConsoleErrors<T>(
      pattern: RegExp,
      action: () => Promise<T>,
      maximum = 1,
    ): Promise<T> {
      if (
        expectedConsole ||
        pattern.global ||
        pattern.sticky ||
        !Number.isInteger(maximum) ||
        maximum < 1 ||
        maximum > 8
      )
        throw new Error("Invalid expected browser-error window");
      expectedConsole = { pattern, remaining: maximum };
      try {
        return await action();
      } finally {
        expectedConsole = null;
      }
    }
    const directory = await mkdtemp(join(tmpdir(), "forge-ui-live-"));
    await chmod(directory, 0o700);
    const control = join(directory, "control.sock");
    const child = launch(
      join(root, "target/debug/forge-ui"),
      [
        "serve",
        "--assets-dir",
        join(root, "frontend/dist-live"),
        "--core-socket",
        core.core_socket,
        "--control-socket",
        control,
      ],
      root,
    );
    let diagnostic = "";
    const record = (chunk: Buffer) => {
      if (diagnostic.length + chunk.length > 65536) child.kill("SIGKILL");
      else diagnostic += chunk.toString();
    };
    child.stdout.on("data", record);
    child.stderr.on("data", record);
    const secrets = new Set<string>();
    let leaked = false;
    async function remember(...values: string[]) {
      for (const value of values) secrets.add(value);
      await registerSecrets(values);
    }
    try {
      const line = await firstLine(child);
      const match = /^Origin: (http:\/\/127\.0\.0\.1:\d+)$/.exec(line);
      if (!match?.[1]) throw new Error("Gateway did not report a loopback origin");
      const origin = match[1];
      async function issue() {
        const issued = await terminalLogin(join(root, "target/debug/forge-cli"), control, root);
        await remember(issued.code);
        if (issued.origin !== origin) throw new Error("Control/HTTP origins differ");
        return issued.code;
      }
      async function bearer() {
        const code = await issue();
        const response = await fetch(`${origin}/api/auth/exchange`, {
          method: "POST",
          headers: { Origin: origin, "Content-Type": "application/json" },
          body: JSON.stringify({ code }),
          signal: AbortSignal.timeout(6000),
        });
        const value: unknown = await response.json();
        const parsed = z.object({ token: z.string().regex(/^[a-f0-9]{64}$/) }).safeParse(value);
        if (!response.ok || !parsed.success) throw new Error("Owner exchange failed");
        await remember(parsed.data.token);
        return parsed.data.token;
      }
      async function login(target: Page) {
        const code = await issue();
        await target.goto(origin);
        try {
          await target.getByLabel("Bootstrap code").fill(code);
          const exchanged = target.waitForResponse(
            (response) =>
              response.url() === `${origin}/api/auth/exchange` &&
              response.request().method() === "POST",
          );
          await target.getByRole("button", { name: "Connect", exact: true }).click();
          const response = await exchanged;
          const value: unknown = await response.json();
          const parsed = z.object({ token: z.string().regex(/^[a-f0-9]{64}$/) }).safeParse(value);
          if (parsed.success) await remember(parsed.data.token);
          if (!response.ok) throw new Error("Owner exchange failed");
        } catch {
          throw new Error("Browser owner login failed");
        }
      }
      await provide({ origin, control, core, secrets, login, bearer, allowConsoleErrors });
    } finally {
      // Strip accidental assertion payloads before Playwright reports them. Close
      // pages before the built-in failure-context capture can snapshot auth input.
      sanitizeErrors(info, secrets);
      try {
        for (const open of page.context().pages())
          await open.close().catch(() => fault("page_close"));
        await stop(child);
        leaked = [...secrets].some((secret) => diagnostic.includes(secret));
        diagnostic = "";
        await rm(directory, { recursive: true, force: true });
      } finally {
        page.context().off("page", watch);
        for (const [opened, listeners] of watched) {
          opened.off("pageerror", listeners.error);
          opened.off("console", listeners.console);
        }
        watched.clear();
      }
    }
    if (leaked) throw new Error("Gateway diagnostics exposed session material");
    if (browserFaults > 0)
      throw new Error(
        `Unexpected browser errors or cleanup failures: ${browserFaults}; categories=${JSON.stringify(Object.fromEntries(faultKinds))}; raw messages withheld`,
      );
  },
});

export { expect };
