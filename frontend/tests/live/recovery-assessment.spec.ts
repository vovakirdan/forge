import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("ineligible Run exposes readiness reason without an acceptance command", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.runs_project.id);
  await page
    .getByRole("navigation", { name: "Project sections" })
    .getByRole("button", { name: "Management" })
    .click();
  const recovery = page.getByRole("region", { name: "Run recovery observations" });
  const run = recovery.getByRole("listitem").first();
  await expect(run).toBeVisible();
  const runId = (await run.textContent())?.match(
    /[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/,
  )?.[0];
  expect(runId).toBeTruthy();
  await run.getByRole("button", { name: "Check assessment readiness" }).click();
  await expect(run).toContainText(/ineligible|Reason:/);
  await expect(run.getByRole("button", { name: "Prepare no-work assessment" })).toHaveCount(0);
  const foreign = await fetch(
    `${live.origin}/api/projects/${live.core.other_runs_project.id}/recovery-runs/${runId}/assessment-readiness`,
    {
      headers: { Authorization: `Bearer ${await live.bearer()}` },
      signal: AbortSignal.timeout(6000),
    },
  );
  expect(foreign.status).toBe(404);
});
