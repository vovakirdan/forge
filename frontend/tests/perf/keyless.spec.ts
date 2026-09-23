import { join } from "node:path";
import { chmod, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { test, expect } from "@playwright/test";
import { z } from "zod";
import { launch, stop, terminalLogin } from "../live/process.ts";
import { sanitizeErrors } from "../live/secrets.ts";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const Readiness = z.object({
  core_socket: z.string().min(1),
  project_id: z.string().uuid(),
  project_name: z.string(),
  tasks: z.literal(1000),
  employees: z.literal(20),
});
let core: ChildProcessWithoutNullStreams | null = null;
let gateway: ChildProcessWithoutNullStreams | null = null;
let fixture: z.infer<typeof Readiness>;
let origin = "";
let control = "";
let directory = "";
const secrets = new Set<string>();

function line(child: ChildProcessWithoutNullStreams, timeoutMs: number): Promise<string> {
  return new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(() => finish(Error("Fixture readiness timeout")), timeoutMs);
    function data(chunk: Buffer) {
      output += chunk.toString();
      if (output.length > 4096) finish(Error("Fixture readiness exceeded bound"));
      else if (output.includes("\n")) finish(undefined, output.split("\n")[0]);
    }
    function exited() {
      finish(Error("Fixture exited before readiness"));
    }
    function finish(error?: Error, value?: string) {
      clearTimeout(timer);
      child.stdout.off("data", data);
      child.off("exit", exited);
      if (error) reject(error);
      else resolve(value ?? "");
    }
    child.stdout.on("data", data);
    child.once("exit", exited);
  });
}

test.beforeAll(async () => {
  test.setTimeout(750_000);
  core = launch("bash", [join(root, "scripts/ui-perf-fixture.sh")], root);
  core.stderr.resume();
  fixture = Readiness.parse(JSON.parse(await line(core, 600_000)));
  directory = await mkdtemp(join(tmpdir(), "forge-ui-perf-"));
  await chmod(directory, 0o700);
  control = join(directory, "control.sock");
  gateway = launch(
    join(root, "target/debug/forge-ui"),
    [
      "serve",
      "--assets-dir",
      join(root, "frontend/dist-live"),
      "--core-socket",
      fixture.core_socket,
      "--control-socket",
      control,
    ],
    root,
  );
  gateway.stderr.resume();
  origin = /^Origin: (http:\/\/127\.0\.0\.1:\d+)$/.exec(await line(gateway, 15_000))?.[1] ?? "";
  if (!origin) throw Error("Gateway readiness failed");
});
test.afterAll(async () => {
  if (gateway) await stop(gateway);
  if (core) await stop(core);
  if (directory) await rm(directory, { recursive: true, force: true });
});
// Playwright requires destructured fixture arguments for hooks.
// eslint-disable-next-line no-empty-pattern
test.afterEach(async ({}, info) => sanitizeErrors(info, secrets));

test("1000 Tasks and 20 Employees stay bounded through the real keyless UI", async ({ page }) => {
  const issued = await terminalLogin(join(root, "target/debug/forge-cli"), control, root);
  secrets.add(issued.code);
  expect(issued.origin).toBe(origin);
  const observations: { kind: "tasks" | "employees"; count: number; bytes: number }[] = [];
  const pending: Promise<void>[] = [];
  page.on("response", (response) => {
    const url = new URL(response.url());
    const kind =
      url.pathname === `/api/projects/${fixture.project_id}/tasks`
        ? "tasks"
        : url.pathname === `/api/projects/${fixture.project_id}/employees`
          ? "employees"
          : null;
    if (!kind || response.status() !== 200) return;
    pending.push(
      (async () => {
        const body = await response.body();
        const value: unknown = JSON.parse(body.toString());
        const page = z.object({ items: z.array(z.unknown()).max(20) }).parse(value);
        observations.push({ kind, count: page.items.length, bytes: body.length });
      })(),
    );
  });
  await page.goto(origin);
  await page.getByLabel("Bootstrap code").fill(issued.code);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  const picker = page.getByRole("region", { name: "Project selector" });
  await picker.getByRole("button", { name: new RegExp(fixture.project_id) }).click();
  const tasks = page.getByRole("region", { name: "Tasks", exact: true });
  await expect(tasks.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(20);
  const start = performance.now();
  for (let next = 2; next <= 50; next++) {
    await tasks.getByRole("button", { name: "Next page", exact: true }).click();
    await expect(tasks).toContainText(`Page ${next}`);
    await expect(tasks.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(20);
  }
  const taskNavigationMs = Math.round(performance.now() - start);
  await expect(tasks.getByRole("button", { name: "Next page", exact: true })).toBeDisabled();
  await tasks.getByRole("button", { name: "Previous page", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(tasks).toContainText("Page 49");
  const sections = page.getByRole("navigation", { name: "Project sections" });
  await sections.getByRole("button", { name: "Team" }).click();
  const team = page.getByRole("region", { name: "Team", exact: true });
  await expect(team.getByRole("list", { name: "Employee list" }).getByRole("listitem")).toHaveCount(
    20,
  );
  await expect(team.getByRole("button", { name: "Next page" })).toBeDisabled();
  await Promise.all(pending);
  const taskPages = observations.filter((item) => item.kind === "tasks");
  const employeePages = observations.filter((item) => item.kind === "employees");
  expect(taskPages.length).toBeGreaterThanOrEqual(50);
  expect(employeePages.length).toBeGreaterThanOrEqual(1);
  expect(observations.every((item) => item.count <= 20 && item.bytes <= 65536)).toBe(true);
  console.log(
    JSON.stringify({
      proof: "UI4_KEYLESS_PERF",
      tasks: fixture.tasks,
      employees: fixture.employees,
      task_pages_observed: taskPages.length,
      employee_pages_observed: employeePages.length,
      max_task_page_bytes: Math.max(...taskPages.map((item) => item.bytes)),
      max_employee_page_bytes: Math.max(...employeePages.map((item) => item.bytes)),
      task_navigation_ms: taskNavigationMs,
    }),
  );
});
