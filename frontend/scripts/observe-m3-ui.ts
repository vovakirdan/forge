/** Owner-browser witness for an already running, isolated M3 session.
 *
 * Usage: node frontend/scripts/observe-m3-ui.ts /tmp/fm3.XXXXXXXX [--once]
 * Writes only allowlisted coordinates and Boolean UI observations to the
 * session's private evidence directory. It never starts a provider Run.
 */
import { readFile, rename, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, type Locator, type Page } from "@playwright/test";
import { z } from "zod";
import { RunListResponseSchema } from "../src/contracts/run.ts";
import { TaskListResponseSchema } from "../src/contracts/task.ts";
import { EmployeeOnboardingStatusSchema } from "../src/contracts/knowledge.ts";
import {
  RunContextCoordinatesSchema,
  RunEvidencePageSchema,
} from "../src/contracts/run-context-evidence.ts";
import { firstLine, launch, stop, terminalLogin } from "../tests/live/process.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const sessionRoot = process.argv[2] ? resolve(process.argv[2]) : "";
const once = process.argv[3] === "--once";
const uuid = z.string().uuid();
const Session = z.object({
  project_id: uuid,
  employee_id: z.union([uuid, z.literal("")]),
  task_id: z.union([uuid, z.literal("")]),
  pipeline_version_id: z.union([uuid, z.literal("")]),
});
type Session = z.infer<typeof Session>;
type RunObservation = {
  id: string;
  task_id: string | null;
  desired_state: string;
  observed_state: string;
  browser_listed: boolean;
  browser_detail: boolean;
  browser_context: boolean;
  context_availability: "available" | "unavailable" | null;
  evidence_receipts: number | null;
};
type Report = {
  status: "observing" | "completed" | "timed_out" | "interrupted" | "failed";
  project_id: string;
  employee_id: string | null;
  task_id: string | null;
  task_lifecycle: string | null;
  onboarding_state: string | null;
  pipeline_version_id: string | null;
  project_visible: boolean;
  employee_visible: boolean;
  task_visible: boolean;
  system_jobs_visible: boolean;
  knowledge_visible: boolean;
  runs: RunObservation[];
  browser_error_count: number;
  failure_code: string | null;
};

if (!sessionRoot || (process.argv[3] && !once) || process.argv.length > 4)
  throw new Error("Usage: node frontend/scripts/observe-m3-ui.ts SESSION_ROOT [--once]");

const reportPath = join(sessionRoot, "evidence", "ui-observer.json");
const report: Report = {
  status: "observing",
  project_id: "",
  employee_id: null,
  task_id: null,
  task_lifecycle: null,
  onboarding_state: null,
  pipeline_version_id: null,
  project_visible: false,
  employee_visible: false,
  task_visible: false,
  system_jobs_visible: false,
  knowledge_visible: false,
  runs: [],
  browser_error_count: 0,
  failure_code: null,
};
let interrupted = false;
for (const signal of ["SIGINT", "SIGTERM"] as const)
  process.once(signal, () => {
    interrupted = true;
  });

async function session(): Promise<Session | null> {
  const contents = await readFile(join(sessionRoot, "session.json"), "utf8").catch(() => null);
  if (contents === null) return null;
  try {
    return Session.safeParse(JSON.parse(contents)).data ?? null;
  } catch {
    return null;
  }
}

async function visible(locator: Locator, timeout = 5000): Promise<boolean> {
  return locator.waitFor({ state: "visible", timeout }).then(
    () => true,
    () => false,
  );
}

async function refresh(page: Page, section: string, button: string): Promise<boolean> {
  const navigation = page.getByRole("navigation", { name: "Project sections" });
  await navigation.getByRole("button", { name: section, exact: true }).click();
  const action = page.getByRole("button", { name: button, exact: true });
  if (!(await visible(action))) return false;
  await action.click();
  return true;
}

async function readRuns(origin: string, token: string, projectId: string) {
  const response = await fetch(`${origin}/api/projects/${projectId}/runs?limit=20`, {
    headers: { Authorization: `Bearer ${token}` },
    signal: AbortSignal.timeout(5000),
  });
  if (response.status !== 200) throw new Error("runs_read_failed");
  const raw: unknown = await response.json();
  const parsed = RunListResponseSchema.safeParse(raw);
  if (!parsed.success) throw new Error("runs_contract_failed");
  return parsed.data;
}

async function readLifecycle(origin: string, token: string, current: Session) {
  if (current.task_id) {
    const response = await fetch(`${origin}/api/projects/${current.project_id}/tasks?limit=20`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(5000),
    });
    if (response.ok) {
      const parsed = TaskListResponseSchema.safeParse(await response.json());
      if (parsed.success)
        report.task_lifecycle =
          parsed.data.items.find((task) => task.id === current.task_id)?.lifecycle ?? null;
    }
  }
  if (current.employee_id) {
    const response = await fetch(
      `${origin}/api/projects/${current.project_id}/employees/${current.employee_id}/onboarding`,
      {
        headers: { Authorization: `Bearer ${token}` },
        signal: AbortSignal.timeout(5000),
      },
    );
    if (response.ok) {
      const parsed = EmployeeOnboardingStatusSchema.safeParse(await response.json());
      if (parsed.success) report.onboarding_state = parsed.data.state;
    }
  }
}

async function readRunEvidence(
  origin: string,
  token: string,
  projectId: string,
  observed: RunObservation,
) {
  const prefix = `${origin}/api/projects/${projectId}/runs/${observed.id}`;
  const headers = { Authorization: `Bearer ${token}` };
  const context = await fetch(`${prefix}/context`, { headers, signal: AbortSignal.timeout(5000) });
  if (context.ok) {
    const parsed = RunContextCoordinatesSchema.safeParse(await context.json());
    if (parsed.success) observed.context_availability = parsed.data.availability;
  }
  const evidence = await fetch(`${prefix}/evidence?limit=20`, {
    headers,
    signal: AbortSignal.timeout(5000),
  });
  if (evidence.ok) {
    const parsed = RunEvidencePageSchema.safeParse(await evidence.json());
    if (parsed.success) observed.evidence_receipts = parsed.data.items.length;
  }
}

async function save() {
  // Atomic replacement permits the launcher to read a complete private report.
  const temporary = `${reportPath}.tmp`;
  await writeFile(temporary, JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
  await rename(temporary, reportPath);
}

const started = Date.now();
let gateway: ReturnType<typeof launch> | undefined;
let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
let page: Page | undefined;
try {
  let current: Session | null = null;
  while (!interrupted && Date.now() - started < 60_000 && !(current = await session()))
    await new Promise((done) => setTimeout(done, 500));
  if (!current) throw new Error("session_unavailable");
  report.project_id = current.project_id;
  const control = join(sessionRoot, "ui-control.sock");
  gateway = launch(
    join(root, "target/debug/forge-ui"),
    [
      "serve",
      "--assets-dir",
      join(root, "frontend/dist-live"),
      "--core-socket",
      join(sessionRoot, "api.sock"),
      "--control-socket",
      control,
    ],
    root,
  );
  gateway.stderr.resume();
  const origin = /^Origin: (http:\/\/127\.0\.0\.1:\d+)$/.exec(await firstLine(gateway))?.[1];
  if (!origin) throw new Error("gateway_unavailable");
  const issued = await terminalLogin(join(root, "target/debug/forge-cli"), control, root);
  if (issued.origin !== origin) throw new Error("login_origin_mismatch");
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  page = await context.newPage();
  page.on("pageerror", () => {
    report.browser_error_count += 1;
  });
  page.on("console", (message) => {
    if (message.type() === "error") report.browser_error_count += 1;
  });
  await page.goto(origin);
  await page.getByLabel("Bootstrap code").fill(issued.code);
  const exchange = page.waitForResponse(
    (response) =>
      response.url() === `${origin}/api/auth/exchange` && response.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  const response = await exchange;
  const exchanged: unknown = await response.json();
  const token = z.object({ token: z.string().regex(/^[a-f0-9]{64}$/) }).safeParse(exchanged)
    .data?.token;
  if (!response.ok() || !token) throw new Error("owner_login_failed");
  if (!(await visible(page.getByText("Connected to Forge", { exact: true }))))
    throw new Error("health_unavailable");
  const picker = page.getByRole("region", { name: "Project selector" });
  let found = false;
  for (let index = 0; index < 10; index++) {
    const project = picker.getByRole("button", { name: new RegExp(current.project_id) });
    if (await visible(project, 1500)) {
      await project.click();
      found = true;
      break;
    }
    const next = picker.getByRole("button", { name: "Next Projects" });
    if (!(await next.isEnabled())) break;
    await next.click();
  }
  if (!found) throw new Error("project_not_listed");
  report.project_visible = await visible(
    page.getByRole("region", { name: "Project", exact: true }).getByText(current.project_id),
  );
  if (!report.project_visible) throw new Error("project_not_selected");
  await save();
  console.log(`UI observer ready: ${origin}`);

  let lastUiCheck = 0;
  while (!interrupted && Date.now() - started < 1_800_000) {
    current = (await session()) ?? current;
    report.employee_id = current.employee_id || null;
    report.task_id = current.task_id || null;
    report.pipeline_version_id = current.pipeline_version_id || null;
    await readLifecycle(origin, token, current).catch(() => {});
    let runs;
    try {
      runs = await readRuns(origin, token, current.project_id);
    } catch {
      // M3 closes its private Core before writing the final report. Allow that
      // bounded shutdown gap, then distinguish completion from a lost Core.
      let finished = false;
      for (let attempt = 0; attempt < 15 && !interrupted; attempt++) {
        finished = await readFile(join(sessionRoot, "evidence", "report.json")).then(
          () => true,
          () => false,
        );
        if (finished) break;
        await new Promise((done) => setTimeout(done, 1000));
      }
      if (finished) {
        report.status = "completed";
        break;
      }
      throw new Error("core_read_lost");
    }
    if (runs.next_cursor || runs.items.length >= 6) {
      report.failure_code = "run_guard_reached";
      report.status = "failed";
      await save();
      break;
    }
    const newRuns = runs.items.filter((run) => !report.runs.some((seen) => seen.id === run.id));
    for (const run of runs.items) {
      const seen = report.runs.find((item) => item.id === run.id);
      if (seen) {
        seen.desired_state = run.desired_state;
        seen.observed_state = run.observed_state;
      } else
        report.runs.push({
          id: run.id,
          task_id: run.task_id,
          desired_state: run.desired_state,
          observed_state: run.observed_state,
          browser_listed: false,
          browser_detail: false,
          browser_context: false,
          context_availability: null,
          evidence_receipts: null,
        });
    }
    for (const observed of report.runs)
      if (observed.context_availability === null || observed.evidence_receipts === null)
        await readRunEvidence(origin, token, current.project_id, observed).catch(() => {});
    if (newRuns.length > 0 || Date.now() - lastUiCheck > 15_000 || once) {
      lastUiCheck = Date.now();
      if (report.employee_id && !report.employee_visible) {
        await refresh(page, "Team", "Refresh team");
        report.employee_visible = await visible(
          page
            .getByRole("region", { name: "Team", exact: true })
            .getByText(report.employee_id, { exact: true }),
          3000,
        );
      }
      if (report.task_id && !report.task_visible) {
        await refresh(page, "Tasks", "Refresh tasks");
        const task = page
          .getByRole("region", { name: "Tasks", exact: true })
          .getByRole("button", { name: /^Open TASK-/ })
          .first();
        if (await visible(task, 3000)) {
          await task.click();
          report.task_visible = await visible(
            page
              .getByRole("region", { name: "Task detail", exact: true })
              .getByText(report.task_id, { exact: true }),
            3000,
          );
        }
      }
      if (!report.system_jobs_visible) {
        await refresh(page, "System Jobs", "Refresh status");
        report.system_jobs_visible = await visible(
          page.getByRole("region", { name: "System Jobs", exact: true }),
          3000,
        );
      }
      if (!report.knowledge_visible) {
        await page
          .getByRole("navigation", { name: "Project sections" })
          .getByRole("button", { name: "Knowledge", exact: true })
          .click();
        report.knowledge_visible = await visible(
          page.getByRole("region", { name: "Knowledge and memory", exact: true }),
          3000,
        );
      }
      await refresh(page, "Runs", "Refresh runs");
      for (const observed of report.runs) {
        const open = page.getByRole("button", { name: `Open Run ${observed.id}`, exact: true });
        observed.browser_listed ||= await visible(open, 3000);
        if (!observed.browser_listed || (observed.browser_detail && observed.browser_context))
          continue;
        await open.click();
        const detail = page.getByRole("region", { name: "Run details", exact: true });
        observed.browser_detail ||= await visible(
          detail.getByText(observed.id, { exact: true }),
          3000,
        );
        observed.browser_context ||= await visible(
          detail.getByRole("region", { name: "Run context and evidence" }),
          3000,
        );
        await detail.getByRole("button", { name: "Close run", exact: true }).click();
      }
    }
    await save();
    if (
      once ||
      (await readFile(join(sessionRoot, "evidence", "report.json")).then(
        () => true,
        () => false,
      ))
    ) {
      report.status = "completed";
      break;
    }
    await new Promise((done) => setTimeout(done, 2000));
  }
  if (interrupted) report.status = "interrupted";
  else if (report.status === "observing") report.status = "timed_out";
} catch (error) {
  report.status = interrupted ? "interrupted" : "failed";
  report.failure_code =
    error instanceof Error && /^[a-z_]+$/.test(error.message) ? error.message : "observer_failed";
} finally {
  if (page)
    await page
      .context()
      .close()
      .catch(() => {});
  if (browser) await browser.close().catch(() => {});
  if (gateway) await stop(gateway).catch(() => {});
  if (report.project_id) await save().catch(() => {});
  console.log(
    `UI observer: ${report.status}${report.failure_code ? ` (${report.failure_code})` : ""}`,
  );
  if (report.status !== "completed") process.exitCode = 1;
}
