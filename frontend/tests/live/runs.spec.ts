import { RunDetailViewSchema, RunListResponseSchema } from "../../src/contracts/run";
import {
  RunContextCoordinatesSchema,
  RunEvidencePageSchema,
} from "../../src/contracts/run-context-evidence";
import { presentRun } from "../../src/presentation/run";
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { expectNoPrivateRunFields, openRuns, runCard, runList } from "./run-helpers";

test("real Run list pages by 20 in server order and detail exposes only the read contract", async ({
  live,
  page,
}) => {
  const project = live.core.runs_project;
  const token = await live.bearer();
  const get = async (tail: string) => {
    const response = await fetch(`${live.origin}/api/projects/${project.id}/${tail}`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(6000),
    });
    expect(response.status).toBe(200);
    const body = await response.text();
    expectNoPrivateRunFields(body, live.secrets);
    return JSON.parse(body) as unknown;
  };
  const first = RunListResponseSchema.parse(await get("runs?limit=20"));
  expect(first.items).toHaveLength(20);
  expect(first.next_cursor).toBeTruthy();
  expect(first.items.every((run) => run.observed_state === "stopped")).toBe(true);
  await openRuns(page, live);
  const list = runList(page);
  await expect(list.getByRole("button", { name: /^Open Run / })).toHaveText(
    first.items.map((run) => `Run ${run.id}`),
  );
  const run = RunDetailViewSchema.parse(await get(`runs/${project.featured_id}`));
  const labels = presentRun(run).presentation;
  await list.getByRole("button", { name: `Open Run ${run.id}`, exact: true }).click();
  const card = runCard(page);
  for (const text of [
    run.id,
    project.featured_task_id,
    project.employee_id,
    labels.purposeLabel,
    labels.desiredStateLabel,
    labels.observedStateLabel,
  ]) {
    await expect(card).toContainText(text);
  }
  for (const label of [
    "Desired state",
    "Observed state",
    "Lease fencing token",
    "Environment epoch",
    "Last observed sequence",
    "RunSpec version",
  ]) {
    await expect(card.getByText(label, { exact: true })).toBeVisible();
  }
  await expect(card.getByRole("region", { name: "Diagnostics", exact: true })).toBeVisible();
  const context = RunContextCoordinatesSchema.parse(
    await get(`runs/${project.featured_id}/context`),
  );
  expect(context.availability).toBe("available");
  if (context.availability === "available") {
    expect(context.coordinates.run_id).toBe(project.featured_id);
    expect(context.coordinates.task_id).toBe(project.featured_task_id);
    expect(context.coordinates.employee_id).toBe(project.employee_id);
  }
  const evidence = RunEvidencePageSchema.parse(
    await get(`runs/${project.featured_id}/evidence?limit=20`),
  );
  expect(evidence.items.every((item) => item.run_id === project.featured_id)).toBe(true);
  await expect(card.getByRole("region", { name: "Run context and evidence" })).toBeVisible();
  if (context.availability === "available")
    await expect(card).toContainText(context.coordinates.stage_id);
  if (evidence.items.length === 0)
    await expect(card).toContainText("No evidence receipts on this page.");
  else await expect(card).toContainText(evidence.items[0]!.id);
  await expect(card.getByRole("link")).toHaveCount(0);
  await expect(card).not.toContainText("deterministic_m0_simulator");
  await list.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open Run / })).toHaveCount(project.total - 20);
  await expect(card).toHaveCount(0);
  await expect(list.getByRole("button", { name: "Next page", exact: true })).toBeDisabled();
  await list.getByRole("button", { name: "Previous page", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open Run / })).toHaveText(
    first.items.map((run) => `Run ${run.id}`),
  );
  await page.reload();
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await expect(runList(page)).toHaveCount(0);
  await expect(card).toHaveCount(0);
});

test("real Run Projects remain isolated, empty Runs are explicit, and section changes reset selection", async ({
  live,
  page,
}) => {
  await openRuns(page, live);
  await page
    .getByRole("button", { name: `Open Run ${live.core.runs_project.featured_id}`, exact: true })
    .click();
  await expect(runCard(page)).toContainText(live.core.runs_project.featured_id);
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await expect(runCard(page)).toHaveCount(0);
  await expect(runList(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(20);
  await expect(runCard(page)).toHaveCount(0);
  await loadProject(page, live.core.other_runs_project.id);
  await expect(page.getByRole("button", { name: "Tasks", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(runList(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(1);
  await expect(runList(page)).toContainText(live.core.other_runs_project.run_id);
  await expect(runList(page)).not.toContainText(live.core.runs_project.featured_id);
  const token = await live.bearer();
  const foreign = await fetch(
    `${live.origin}/api/projects/${live.core.other_runs_project.id}/runs/${live.core.runs_project.featured_id}`,
    {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(6000),
    },
  );
  expect(foreign.status).toBe(404);
  const cursor = await fetch(
    `${live.origin}/api/projects/${live.core.runs_project.id}/runs?limit=20&cursor=missing`,
    {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(6000),
    },
  );
  expect(cursor.status).toBe(409);
  expect(await cursor.json()).toMatchObject({ code: "cursor_invalid" });
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(runList(page)).toContainText("No runs in this project.");
  await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(0);
});

test("Run refresh offline preserves stale details and a keyboard retry works at 375px", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openRuns(page, live);
  const open = page.getByRole("button", {
    name: `Open Run ${live.core.runs_project.featured_id}`,
    exact: true,
  });
  await open.focus();
  await page.keyboard.press("Enter");
  const card = runCard(page);
  await expect(card).toContainText(live.core.runs_project.featured_id);
  await page.context().setOffline(true);
  await live.allowConsoleErrors(
    /^Failed to load resource: net::ERR_INTERNET_DISCONNECTED$/,
    async () => {
      const failed = page.waitForEvent("console", {
        predicate: (message) =>
          message.text() === "Failed to load resource: net::ERR_INTERNET_DISCONNECTED",
      });
      await card.getByRole("button", { name: "Refresh run", exact: true }).click();
      await expect(card).toContainText(/showing stale Run details/i);
      await expect(card).toContainText(live.core.runs_project.featured_id);
      await failed;
    },
  );
  await page.context().setOffline(false);
  const recovered = page.waitForResponse(
    (response) =>
      response.url() ===
        `${live.origin}/api/projects/${live.core.runs_project.id}/runs/${live.core.runs_project.featured_id}` &&
      response.status() === 200,
  );
  await card.getByRole("button", { name: "Refresh run", exact: true }).click();
  await recovered;
  await expect(card.getByRole("button", { name: "Refresh run", exact: true })).toBeEnabled();
  await expect(card).not.toContainText(/showing stale/i);
  await expect(card).toContainText(live.core.runs_project.employee_id);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  const close = card.getByRole("button", { name: "Close run", exact: true });
  await close.focus();
  await page.keyboard.press("Enter");
  await expect(card).toHaveCount(0);
});
