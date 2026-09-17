import { test, expect } from "./fixtures";
import { holdRealResponse, loadProject, openTasks } from "./task-helpers";

test("switching Task cancels a delayed pinned Pipeline without overwriting the next stage", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${live.core.project_id}/pipelines/${live.core.tasks.pinned_version_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    const card = page.getByRole("region", { name: "Task detail", exact: true });
    await expect(card).toContainText(live.core.tasks.featured_title);
    await page
      .getByRole("button", { name: `Open ${live.core.tasks.second_key}`, exact: true })
      .click();
    await expect(card).toContainText(live.core.tasks.second_id);
    await held.releaseAfterCancellation();
    await expect(card).not.toContainText(live.core.tasks.stage_name);
    await expect(card).toContainText(live.core.tasks.second_id);
  } finally {
    await held.cleanup();
  }
});

test("switching Task cancels a delayed real detail without restoring the previous selection", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${live.core.project_id}/tasks/${live.core.tasks.featured_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    await page
      .getByRole("button", { name: `Open ${live.core.tasks.second_key}`, exact: true })
      .click();
    const card = page.getByRole("region", { name: "Task detail", exact: true });
    await expect(card).toContainText(live.core.tasks.second_id);
    await held.releaseAfterCancellation();
    await expect(card).toContainText(live.core.tasks.second_id);
    await expect(card).not.toContainText(live.core.tasks.featured_title);
  } finally {
    await held.cleanup();
  }
});

test("switching Project cancels a delayed real Task list and resets pagination", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${live.core.project_id}/tasks?limit=20`,
  );
  try {
    await page.getByRole("button", { name: "Refresh tasks", exact: true }).click();
    await held.ready;
    held.assertCaptured();
    await loadProject(page, live.core.second_project.id);
    const list = page.getByRole("region", { name: "Tasks", exact: true });
    await expect(list).toContainText(live.core.second_project.task_title);
    await held.releaseAfterCancellation();
    await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(1);
    await expect(list).not.toContainText(live.core.tasks.featured_title);
  } finally {
    await held.cleanup();
  }
});

test("logout cancels a delayed real Task read and clears selected data", async ({ live, page }) => {
  await openTasks(page, live);
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${live.core.project_id}/tasks/${live.core.tasks.featured_id}`,
  );
  try {
    await page
      .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    await page.getByRole("button", { name: "Log out", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Connect to Forge", exact: true }),
    ).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(page.getByRole("region", { name: "Tasks", exact: true })).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Task detail", exact: true })).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
});

test("real server revocation during Task read clears the active UI session", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  // The new tab copies the same session; its normal logout revokes that session.
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
      await page
        .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Connect to Forge", exact: true }),
      ).toBeVisible();
    },
  );
  await expect(page.getByRole("region", { name: "Tasks", exact: true })).toHaveCount(0);
});
