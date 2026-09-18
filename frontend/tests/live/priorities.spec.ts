import { test, expect } from "./fixtures";
import { commandClient, draftForm, fixtureDraft } from "./draft-helpers";
import { loadProject, holdRealResponse } from "./task-helpers";
import { fact } from "./pipeline-helpers";

test("real priorities share one read across Task rows and detail without changing the stopped Project", async ({
  live,
  page,
}) => {
  const client = await commandClient(live);
  const before = await client.project();
  const draft = fixtureDraft(live, 11);
  const taskBefore = await client.task(draft.id);
  const path = `/api/projects/${live.core.commands_project.id}/priority-scheme`;
  let reads = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(path)) reads += 1;
  });
  await live.login(page);
  const response = page.waitForResponse((value) => value.url().endsWith(path));
  await loadProject(page, live.core.commands_project.id);
  expect(await (await response).json()).toEqual({
    project_id: before.id,
    project_revision: before.revision,
    default_level_id: "normal",
    levels: [
      { id: "high", display_name: "High", rank: 100, retired: false },
      { id: "low", display_name: "Low", rank: 10, retired: false },
      { id: "normal", display_name: "Normal", rank: 50, retired: false },
    ],
  });
  const row = page
    .getByRole("list", { name: "Task list", exact: true })
    .getByRole("listitem")
    .filter({
      has: page.getByRole("button", { name: `Open ${draft.key}`, exact: true }),
    });
  await fact(row, "Priority ID", "normal");
  await fact(row, "Priority", "Normal");
  await row.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await fact(draftForm(page), "Priority", "Normal");
  expect(reads).toBe(1);
  await page.getByRole("button", { name: "Refresh priorities", exact: true }).click();
  await expect(page.getByRole("button", { name: "Refresh priorities", exact: true })).toBeEnabled();
  expect(reads).toBe(2);
  expect(await client.project()).toEqual(before);
  expect(await client.task(draft.id)).toEqual(taskBefore);
  expect((await client.runs()).items).toEqual([]);
  expect(before.execution_gate).toBe("stopped");
  await loadProject(page, live.core.second_project.id);
  const secondRow = page
    .getByRole("list", { name: "Task list", exact: true })
    .getByRole("listitem");
  await expect(secondRow).toHaveCount(1);
  await fact(secondRow, "Priority", "Normal");
  await fact(secondRow, "Priority ID", "normal");
});

test("delayed real priority read is discarded on Project switch", async ({ live, page }) => {
  const url = `${live.origin}/api/projects/${live.core.commands_project.id}/priority-scheme`;
  const held = await holdRealResponse(page, url);
  try {
    await live.login(page);
    await loadProject(page, live.core.commands_project.id);
    await held.ready;
    held.assertCaptured();
    await expect(page.getByText("Loading priorities…", { exact: true })).toBeVisible();
    await loadProject(page, live.core.second_project.id);
    await held.releaseAfterCancellation();
    const rows = page.getByRole("list", { name: "Task list", exact: true }).getByRole("listitem");
    await expect(rows).toHaveCount(1);
    await fact(rows, "Priority", "Normal");
    await expect(page.getByText("Loading priorities…", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Project", exact: true })).toContainText(
      live.core.second_project.id,
    );
  } finally {
    await held.cleanup();
  }
});

test("logout cancels a delayed priority read and a new session reads anew", async ({
  live,
  page,
}) => {
  const url = `${live.origin}/api/projects/${live.core.commands_project.id}/priority-scheme`;
  const held = await holdRealResponse(page, url);
  try {
    await live.login(page);
    await loadProject(page, live.core.commands_project.id);
    await held.ready;
    held.assertCaptured();
    await page.getByRole("button", { name: "Log out", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Connect to Forge", exact: true }),
    ).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(page.getByRole("region", { name: "Tasks", exact: true })).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
  await live.login(page);
  await loadProject(page, live.core.second_project.id);
  await fact(
    page.getByRole("list", { name: "Task list", exact: true }).getByRole("listitem"),
    "Priority",
    "Normal",
  );
});
