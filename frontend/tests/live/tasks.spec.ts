import { test, expect } from "./fixtures";
import { loadProject, openTasks } from "./task-helpers";
import { TaskDetailViewSchema, TaskListResponseSchema } from "../../src/contracts/task";
import { PipelineVersionViewSchema } from "../../src/contracts/pipeline";

test("real Task list pages by 20 and the card resolves its soft-deleted pinned Pipeline", async ({
  live,
  page,
}) => {
  const paths: string[] = [];
  const origins: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    paths.push(url.pathname);
    origins.push(url.origin);
  });
  await openTasks(page, live);
  const list = page.getByRole("region", { name: "Tasks", exact: true });
  await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(20);
  expect(paths.filter((path) => path.includes("/pipelines/"))).toHaveLength(0);
  const token = await live.bearer();
  const get = async (path: string) => {
    const response = await fetch(`${live.origin}/api/projects/${live.core.project_id}/${path}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(response.status).toBe(200);
    return response.json();
  };
  const first = TaskListResponseSchema.parse(await get("tasks?limit=20"));
  expect(first.items).toHaveLength(20);
  expect(first.next_cursor).toBeTruthy();
  for (const task of first.items) await expect(list).toContainText(task.title);
  const task = TaskDetailViewSchema.parse(await get(`tasks/${live.core.tasks.featured_id}`));
  await list.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
  const card = page.getByRole("region", { name: "Task detail", exact: true });
  await expect(card).toContainText(task.title);
  await expect(card).toContainText(task.description);
  await expect(card).toContainText(task.definition_of_done ?? "");
  await expect(card).toContainText(live.core.tasks.stage_name);
  await expect(card).toContainText(live.core.tasks.pinned_version_id);
  for (const artifact of task.artifacts) {
    await expect(card).toContainText(artifact.id);
    await expect(card).toContainText(artifact.title);
  }
  expect(task.wait_conditions.length).toBeGreaterThan(0);
  for (const wait of task.wait_conditions) {
    await expect(card).toContainText(wait.kind);
    if (wait.detail) await expect(card).toContainText(wait.detail);
  }
  const pinned = PipelineVersionViewSchema.parse(
    await get(`pipelines/${task.pipeline_version_id}`),
  );
  expect(pinned.version).toBe(1);
  expect(pinned.default_version_id).toBe(live.core.tasks.default_version_id);
  expect(pinned.default_version_id).not.toBe(pinned.id);
  expect(pinned.deleted_at).not.toBeNull();
  expect(await page.evaluate(() => "forgeInjected" in window)).toBe(false);
  expect(origins.every((origin) => origin === live.origin)).toBe(true);
  await expect(card).not.toContainText("evidence.invalid");
  await expect(card.getByRole("link")).toHaveCount(0);
  expect(paths.filter((path) => path.includes("/pipelines/"))).toEqual([
    `/api/projects/${live.core.project_id}/pipelines/${task.pipeline_version_id}`,
  ]);
  await list.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(
    live.core.tasks.total - 20,
  );
  await expect(card).toHaveCount(0);
  await expect(list.getByRole("button", { name: "Next page", exact: true })).toBeDisabled();
  await list.getByRole("button", { name: "Previous page", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(20);
  await list
    .getByRole("button", { name: `Open ${live.core.tasks.draft_key}`, exact: true })
    .click();
  await expect(card).toContainText(/no current stage/i);
  await expect(card).toContainText("contains_migrations");
  await expect(card).toContainText("Needs an owner-defined property schema");
  await page.reload();
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Tasks", exact: true })).toHaveCount(0);
  await expect(card).toHaveCount(0);
});

test("real Project scope stays separate, empty is explicit and foreign Task/Pipeline reads fail", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  await page
    .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
    .click();
  await expect(page.getByRole("region", { name: "Task detail", exact: true })).toContainText(
    live.core.tasks.featured_title,
  );
  await loadProject(page, live.core.second_project.id);
  const list = page.getByRole("region", { name: "Tasks", exact: true });
  await expect(list).toContainText(live.core.second_project.task_title);
  await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(1);
  await expect(page.getByRole("region", { name: "Task detail", exact: true })).toHaveCount(0);
  await expect(list).not.toContainText(live.core.tasks.featured_title);
  const token = await live.bearer();
  for (const tail of [
    `tasks/${live.core.tasks.featured_id}`,
    `pipelines/${live.core.tasks.pinned_version_id}`,
  ]) {
    const response = await fetch(
      `${live.origin}/api/projects/${live.core.second_project.id}/${tail}`,
      {
        headers: { Authorization: `Bearer ${token}` },
      },
    );
    expect(response.status).toBe(404);
  }
  await loadProject(page, live.core.empty_project.id);
  await expect(list).toContainText(/no tasks/i);
  await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(0);
});

test("Task refresh offline preserves stale data and retry recovers; the card works with keyboard on a narrow screen", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openTasks(page, live);
  const open = page.getByRole("button", {
    name: `Open ${live.core.tasks.featured_key}`,
    exact: true,
  });
  await open.focus();
  await page.keyboard.press("Enter");
  const card = page.getByRole("region", { name: "Task detail", exact: true });
  await expect(card).toContainText(live.core.tasks.stage_name);
  await page.context().setOffline(true);
  await live.allowConsoleErrors(
    /^Failed to load resource: net::ERR_INTERNET_DISCONNECTED$/,
    async () => {
      await card.getByRole("button", { name: "Refresh task", exact: true }).click();
      await expect(card).toContainText(/stale/i);
      await expect(card).toContainText(live.core.tasks.featured_title);
    },
  );
  await page.context().setOffline(false);
  await card.getByRole("button", { name: "Refresh task", exact: true }).click();
  await expect(card).not.toContainText(/stale/i);
  await expect(card).toContainText(live.core.tasks.stage_name);
  await page.context().setOffline(true);
  await live.allowConsoleErrors(
    /^Failed to load resource: net::ERR_INTERNET_DISCONNECTED$/,
    async () => {
      // Refreshing the same Project must not destroy the already selected Task.
      await page.getByRole("button", { name: "Refresh project", exact: true }).click();
      await expect(page.getByRole("region", { name: "Project error", exact: true })).toContainText(
        /stale/i,
      );
      await expect(card).toContainText(live.core.tasks.featured_title);
    },
  );
  await page.context().setOffline(false);
  await page.getByRole("button", { name: "Retry project", exact: true }).click();
  await expect(page.getByRole("region", { name: "Project error", exact: true })).toHaveCount(0);
  await expect(card).toContainText(live.core.tasks.featured_title);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  const close = card.getByRole("button", { name: "Close task", exact: true });
  await close.focus();
  await page.keyboard.press("Enter");
  await expect(card).toHaveCount(0);
});

test("real invalid cursor returns a safe finite code, not raw Core diagnostics", async ({
  live,
}) => {
  const token = await live.bearer();
  const response = await fetch(
    `${live.origin}/api/projects/${live.core.project_id}/tasks?limit=20&cursor=missing`,
    {
      headers: { Authorization: `Bearer ${token}` },
    },
  );
  expect(response.status).toBe(409);
  expect(await response.json()).toMatchObject({ code: "cursor_invalid" });
});
