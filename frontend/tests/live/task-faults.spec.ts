// Fault injections test UI recovery, not Core behavior. Happy path data still
// comes from the real named-command fixture through the authenticated gateway.
import { test, expect } from "./fixtures";
import { openTasks } from "./task-helpers";

test("synthetic expired pagination cursor offers first-page recovery", async ({ live, page }) => {
  await openTasks(page, live);
  await page.route(`**/api/projects/${live.core.project_id}/tasks?limit=20&cursor=*`, (route) =>
    route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: "Cursor is no longer available", code: "cursor_invalid" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/,
    async () => {
      await page.getByRole("button", { name: "Next page", exact: true }).click();
      await expect(
        page.getByRole("button", { name: "Restart pagination", exact: true }),
      ).toBeVisible();
    },
  );
  await page.getByRole("button", { name: "Restart pagination", exact: true }).click();
  await expect(
    page
      .getByRole("region", { name: "Tasks", exact: true })
      .getByRole("button", { name: /^Open TASK-/ }),
  ).toHaveCount(20);
});

test("synthetic list failure is not an empty Project and a successful retry restores real Tasks", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  const url = `${live.origin}/api/projects/${live.core.project_id}/tasks?limit=20`;
  await page.route(url, (route) =>
    route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({ error: "Unavailable" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 \(Service Unavailable\)$/,
    async () => {
      await page
        .getByRole("list", { name: "Projects" })
        .getByRole("button", { name: new RegExp(live.core.project_id) })
        .click();
      await expect(
        page.getByRole("region", { name: "Tasks", exact: true }).getByRole("alert"),
      ).toBeVisible();
    },
  );
  const list = page.getByRole("region", { name: "Tasks", exact: true });
  await expect(list).not.toContainText(/no tasks/i);
  await page.unroute(url);
  await list.getByRole("button", { name: "Refresh tasks", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(20);
  await page.route(url, (route) =>
    route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({ error: "Unavailable" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 \(Service Unavailable\)$/,
    async () => {
      await list.getByRole("button", { name: "Refresh tasks", exact: true }).click();
      await expect(list).toContainText(/showing stale tasks/i);
      await expect(list.getByRole("button", { name: /^Open TASK-/ })).toHaveCount(20);
    },
  );
});

test("synthetic missing selected Task does not become an empty or successful card", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  await page.route(`**/tasks/${live.core.tasks.featured_id}`, (route) =>
    route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ error: "Not found" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 404 \(Not Found\)$/,
    async () => {
      await page
        .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
        .click();
      const card = page.getByRole("region", { name: "Task detail", exact: true });
      await expect(card.getByRole("alert")).toContainText("Task not found.");
      await expect(card.getByRole("region", { name: "Artifacts", exact: true })).toHaveCount(0);
    },
  );
});

test("synthetic oversized detail is explicit, not a corrupt Task or a partial card", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  await page.route(`**/tasks/${live.core.tasks.featured_id}`, (route) =>
    route.fulfill({
      status: 502,
      contentType: "application/json",
      body: JSON.stringify({
        error: "Response exceeds interface limit",
        code: "response_too_large",
      }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 502 \(Bad Gateway\)$/,
    async () => {
      await page
        .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
        .click();
      await expect(
        page.getByRole("region", { name: "Task detail", exact: true }).getByRole("alert"),
      ).toContainText(/limit|too large/i);
    },
  );
  await expect(page.getByRole("region", { name: "Task detail", exact: true })).not.toContainText(
    /corrupt|invalid response/i,
  );
});

test("synthetic Pipeline failure keeps real Task fields and identifies the unresolved stage", async ({
  live,
  page,
}) => {
  await openTasks(page, live);
  await page.route(`**/pipelines/${live.core.tasks.pinned_version_id}`, (route) =>
    route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({ error: "Unavailable" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 \(Service Unavailable\)$/,
    async () => {
      await page
        .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
        .click();
      const card = page.getByRole("region", { name: "Task detail", exact: true });
      await expect(card).toContainText(live.core.tasks.featured_title);
      await expect(card).toContainText(live.core.tasks.stage_id);
      await expect(card).toContainText(/unavailable/i);
    },
  );
});

test("synthetic stale summary pin never overrides the fresh real Task detail pin", async ({
  live,
  page,
}) => {
  const pipelineRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/pipelines/")) pipelineRequests.push(request.url());
  });
  await page.route(`**/api/projects/${live.core.project_id}/tasks?limit=20`, async (route) => {
    // Alter one field to emulate stale list data; detail and Pipeline stay real.
    try {
      const response = await route.fetch({ timeout: 10_000 });
      const value: unknown = await response.json();
      const { TaskListResponseSchema } = await import("../../src/contracts/task");
      const list = TaskListResponseSchema.parse(value);
      const items = list.items.map((task) =>
        task.id === live.core.tasks.featured_id
          ? { ...task, pipeline_version_id: live.core.tasks.default_version_id }
          : task,
      );
      await route.fulfill({ response, json: { ...list, items } });
    } catch {
      throw new Error("Stale summary injection failed; request diagnostics withheld");
    }
  });
  await openTasks(page, live);
  await page
    .getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true })
    .click();
  await expect(page.getByRole("region", { name: "Task detail", exact: true })).toContainText(
    live.core.tasks.stage_name,
  );
  expect(pipelineRequests).toEqual([
    `${live.origin}/api/projects/${live.core.project_id}/pipelines/${live.core.tasks.pinned_version_id}`,
  ]);
});
