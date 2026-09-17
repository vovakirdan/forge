import { test, expect } from "./fixtures";
import { holdRealResponse, loadProject } from "./task-helpers";
import { openRuns, runCard, runList } from "./run-helpers";

test("switching Run cancels a delayed real detail without restoring the previous selection", async ({
  live,
  page,
}) => {
  await openRuns(page, live);
  const project = live.core.runs_project;
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${project.id}/runs/${project.featured_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open Run ${project.featured_id}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    await page.getByRole("button", { name: `Open Run ${project.second_id}`, exact: true }).click();
    await expect(runCard(page)).toContainText(project.second_id);
    await held.releaseAfterCancellation();
    await expect(runCard(page)).toContainText(project.second_id);
    await expect(runCard(page)).not.toContainText(project.featured_id);
  } finally {
    await held.cleanup();
  }
});

test("switching Runs to Tasks cancels a delayed detail and discards Run selection", async ({
  live,
  page,
}) => {
  await openRuns(page, live);
  const project = live.core.runs_project;
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${project.id}/runs/${project.featured_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open Run ${project.featured_id}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    await page.getByRole("button", { name: "Tasks", exact: true }).click();
    await expect(page.getByRole("region", { name: "Tasks", exact: true })).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(runList(page)).toHaveCount(0);
    await expect(runCard(page)).toHaveCount(0);
    await page.getByRole("button", { name: "Runs", exact: true }).click();
    await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(20);
    await expect(runCard(page)).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
});

test("switching Project cancels a delayed real Run list and never requests the old Project again", async ({
  live,
  page,
}) => {
  await openRuns(page, live);
  const url = `${live.origin}/api/projects/${live.core.runs_project.id}/runs?limit=20`;
  let oldRequests = 0;
  page.on("request", (request) => {
    if (request.url() === url) oldRequests += 1;
  });
  const held = await holdRealResponse(page, url);
  try {
    await runList(page).getByRole("button", { name: "Refresh runs", exact: true }).click();
    await held.ready;
    held.assertCaptured();
    await loadProject(page, live.core.other_runs_project.id);
    await expect(page.getByRole("button", { name: "Tasks", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await page.getByRole("button", { name: "Runs", exact: true }).click();
    await expect(runList(page)).toContainText(live.core.other_runs_project.run_id);
    await held.releaseAfterCancellation();
    await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(1);
    await expect(runList(page)).not.toContainText(live.core.runs_project.featured_id);
    expect(oldRequests, "Scope switch must not recreate the previous observed query").toBe(1);
  } finally {
    await held.cleanup();
  }
});

test("logout cancels a delayed real Run read and clears selected data", async ({ live, page }) => {
  await openRuns(page, live);
  const project = live.core.runs_project;
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${project.id}/runs/${project.featured_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open Run ${project.featured_id}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    await page.getByRole("button", { name: "Log out", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Connect to Forge", exact: true }),
    ).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(runList(page)).toHaveCount(0);
    await expect(runCard(page)).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
});

test("real 401 revocation cancels a pending Run detail and removes the session UI", async ({
  live,
  page,
}) => {
  await openRuns(page, live);
  const project = live.core.runs_project;
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${project.id}/runs/${project.featured_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open Run ${project.featured_id}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    const popup = page.waitForEvent("popup");
    await page.evaluate(() => {
      window.open(window.location.href, "_blank");
    });
    const copied = await popup;
    await expect(copied.getByText("Connected to Forge", { exact: true })).toBeVisible();
    const revoked = copied.waitForResponse(
      (response) => response.url() === `${live.origin}/api/auth/logout`,
    );
    await copied.getByRole("button", { name: "Log out", exact: true }).click();
    expect((await revoked).status()).toBe(204);
    await live.allowConsoleErrors(
      /^Failed to load resource: the server responded with a status of 401 \(Unauthorized\)$/,
      async () => {
        await runList(page).getByRole("button", { name: "Refresh runs", exact: true }).click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      },
    );
    await held.releaseAfterCancellation();
    await expect(runList(page)).toHaveCount(0);
    await expect(runCard(page)).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
});
