// Deliberately synthetic transport faults prove UI recovery, not Core execution.
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { openRuns, runCard, runList } from "./run-helpers";

test("synthetic Run cursor expiration offers first-page recovery", async ({ live, page }) => {
  await openRuns(page, live);
  await page.route(`**/api/projects/${live.core.runs_project.id}/runs?limit=20&cursor=*`, (route) =>
    route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: "Cursor expired", code: "cursor_invalid" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/,
    async () => {
      await runList(page).getByRole("button", { name: "Next page", exact: true }).click();
      await expect(
        runList(page).getByRole("button", { name: "Restart pagination", exact: true }),
      ).toBeVisible();
    },
  );
  await runList(page).getByRole("button", { name: "Restart pagination", exact: true }).click();
  await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(20);
});

test("synthetic Run list failure is not empty and stale retry preserves real entries", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.runs_project.id);
  const url = `${live.origin}/api/projects/${live.core.runs_project.id}/runs?limit=20`;
  const unavailable = (route: import("@playwright/test").Route) =>
    route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({ error: "Unavailable" }),
    });
  await page.route(url, unavailable);
  const error =
    /^Failed to load resource: the server responded with a status of 503 \(Service Unavailable\)$/;
  await live.allowConsoleErrors(error, async () => {
    await page.getByRole("button", { name: "Runs", exact: true }).click();
    await expect(runList(page).getByRole("alert")).toBeVisible();
    await expect(runList(page)).not.toContainText("No runs in this project.");
  });
  await page.unroute(url);
  await runList(page).getByRole("button", { name: "Refresh runs", exact: true }).click();
  await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(20);
  await page.route(url, unavailable);
  await live.allowConsoleErrors(error, async () => {
    await runList(page).getByRole("button", { name: "Refresh runs", exact: true }).click();
    await expect(runList(page)).toContainText(/showing stale runs/i);
    await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(20);
  });
});

for (const failure of [
  {
    name: "missing",
    status: 404,
    console: "Not Found",
    code: undefined,
    message: /Run not found/i,
  },
  {
    name: "oversized",
    status: 502,
    console: "Bad Gateway",
    code: "response_too_large",
    message: /limit|too large/i,
  },
] as const) {
  test(`synthetic ${failure.name} Run detail stays explicit without a partial successful card`, async ({
    live,
    page,
  }) => {
    await openRuns(page, live);
    const url = `${live.origin}/api/projects/${live.core.runs_project.id}/runs/${live.core.runs_project.featured_id}`;
    await page.route(url, (route) =>
      route.fulfill({
        status: failure.status,
        contentType: "application/json",
        body: JSON.stringify({ error: "Unavailable", code: failure.code }),
      }),
    );
    const error = new RegExp(
      `^Failed to load resource: the server responded with a status of ${failure.status} \\(${failure.console}\\)$`,
    );
    await live.allowConsoleErrors(error, async () => {
      await page
        .getByRole("button", {
          name: `Open Run ${live.core.runs_project.featured_id}`,
          exact: true,
        })
        .click();
      await expect(runCard(page).getByRole("alert")).toContainText(failure.message);
      await expect(
        runCard(page).getByRole("region", { name: "Diagnostics", exact: true }),
      ).toHaveCount(0);
    });
    await page.unroute(url);
    await runCard(page).getByRole("button", { name: "Refresh run", exact: true }).click();
    await expect(runCard(page)).toContainText(live.core.runs_project.featured_id);
    await expect(
      runCard(page).getByRole("region", { name: "Diagnostics", exact: true }),
    ).toBeVisible();
  });
}
