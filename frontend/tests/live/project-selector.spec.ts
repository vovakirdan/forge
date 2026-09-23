import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { ProjectListResponseSchema } from "../../src/contracts/project";

test("Project selector reads a later real page and keeps scopes separate", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await live.login(page);
  const picker = page.getByRole("region", { name: "Project selector" });
  const list = picker.getByRole("list", { name: "Projects" });
  await expect(list.getByRole("button")).toHaveCount(20);
  const token = await live.bearer();
  const firstResponse = await fetch(`${live.origin}/api/projects?limit=20`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(firstResponse.status).toBe(200);
  const first = ProjectListResponseSchema.parse(await firstResponse.json());
  expect(first.items).toHaveLength(20);
  expect(first.next_cursor).toBeTruthy();
  expect(first.items.some((item) => item.id === live.core.later_project.id)).toBe(false);
  await picker.getByRole("button", { name: "Next Projects" }).click();
  await expect(picker).toContainText("Page 2");
  const later = list.getByRole("button", { name: new RegExp(live.core.later_project.id) });
  await expect(later).toBeVisible();
  await later.focus();
  await page.keyboard.press("Enter");
  const project = page.getByRole("region", { name: "Project", exact: true });
  await expect(project).toContainText(live.core.later_project.name);
  await loadProject(page, live.core.project_id);
  await expect(project).toContainText(live.core.project_name);
  await expect(project).not.toContainText(live.core.later_project.id);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.reload();
  await expect(picker).toBeVisible();
  await expect(project).toHaveCount(0);
});

test("Project catalog rejects stale cursors and the selector can restart", async ({
  live,
  page,
}) => {
  const token = await live.bearer();
  const stale = await fetch(`${live.origin}/api/projects?limit=20&cursor=missing`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(stale.status).toBe(409);
  expect(await stale.json()).toMatchObject({ code: "cursor_invalid" });
  await live.login(page);
  await page.route("**/api/projects?limit=20&cursor=*", (route) =>
    route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: "Expired", code: "cursor_invalid" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/,
    async () => {
      await page.getByRole("button", { name: "Next Projects" }).click();
      await expect(page.getByRole("button", { name: "Restart Project pages" })).toBeVisible();
    },
  );
  await page.getByRole("button", { name: "Restart Project pages" }).click();
  await expect(page.getByRole("list", { name: "Projects" }).getByRole("button")).toHaveCount(20);
});

test("an empty Project page never becomes a demo selection", async ({ live, page }) => {
  const url = `${live.origin}/api/projects?limit=20`;
  await page.route(url, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: '{"items":[]}' }),
  );
  await live.login(page);
  const picker = page.getByRole("region", { name: "Project selector" });
  await expect(picker).toContainText("No Projects on this page.");
  await expect(page.getByRole("region", { name: "Project", exact: true })).toHaveCount(0);
  await page.unroute(url);
  await picker.getByRole("button", { name: "Refresh Projects" }).click();
  await expect(picker.getByRole("list", { name: "Projects" }).getByRole("button")).toHaveCount(20);
});
