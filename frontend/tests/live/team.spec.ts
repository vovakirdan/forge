import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { EmployeeListResponseSchema } from "../../src/contracts/employee";

test("Team reads scoped canonical roster in stable pages without availability claims", async ({
  live,
  page,
}) => {
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const path = `${live.origin}/api/projects/${live.core.team_project.id}/employees`;
  const firstResponse = await fetch(`${path}?limit=20`, { headers });
  expect(firstResponse.status).toBe(200);
  const first = EmployeeListResponseSchema.parse(await firstResponse.json());
  expect(first.items).toHaveLength(20);
  expect(first.next_cursor).toBeTruthy();
  const secondResponse = await fetch(`${path}?limit=20&cursor=${first.next_cursor}`, { headers });
  expect(secondResponse.status).toBe(200);
  const second = EmployeeListResponseSchema.parse(await secondResponse.json());
  expect(second.items).toHaveLength(2);
  expect(second.next_cursor).toBeUndefined();
  const all = [...first.items, ...second.items];
  expect(all.map((item) => [item.name, item.id])).toEqual(
    [...all]
      .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
      .map((item) => [item.name, item.id]),
  );
  expect(new Set(all.map((item) => item.name)).size).toBe(22);
  expect(new Set(all.map((item) => item.state))).toEqual(
    new Set(["enabled", "disabled", "retired"]),
  );
  expect(
    all.every(
      (item) =>
        Object.keys(item).sort().join() === "id,max_concurrent_runs,name,revision,role,state",
    ),
  ).toBe(true);
  const foreign = await fetch(
    `${live.origin}/api/projects/${live.core.empty_project.id}/employees?limit=20`,
    { headers },
  );
  expect(foreign.status).toBe(200);
  expect(EmployeeListResponseSchema.parse(await foreign.json()).items).toHaveLength(0);
  const stale = await fetch(`${path}?limit=20&cursor=missing`, { headers });
  expect(stale.status).toBe(409);
  expect(await stale.json()).toMatchObject({ code: "cursor_invalid" });

  await page.setViewportSize({ width: 375, height: 812 });
  await live.login(page);
  await loadProject(page, live.core.team_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  const team = page.getByRole("region", { name: "Team" });
  await expect(team.getByRole("list", { name: "Employee list" }).getByRole("listitem")).toHaveCount(
    20,
  );
  await expect(team).toContainText("Page 1");
  await team.getByRole("button", { name: "Next page" }).click();
  await expect(team.getByRole("list", { name: "Employee list" }).getByRole("listitem")).toHaveCount(
    2,
  );
  await expect(team).toContainText("Page 2");
  await team.getByRole("button", { name: "Previous page" }).focus();
  await page.keyboard.press("Enter");
  await expect(team.getByRole("list", { name: "Employee list" }).getByRole("listitem")).toHaveCount(
    20,
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await expect(page.getByRole("region", { name: "Team" })).toContainText(
    "No Employees in this project.",
  );
});

test("Team restarts invalid pagination and refreshes an empty page", async ({ live, page }) => {
  await live.login(page);
  await loadProject(page, live.core.team_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  const team = page.getByRole("region", { name: "Team" });
  await expect(team.getByRole("button", { name: "Next page" })).toBeEnabled();
  await page.route("**/employees?limit=20&cursor=*", (route) =>
    route.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: "Expired", code: "cursor_invalid" }),
    }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/,
    async () => {
      await team.getByRole("button", { name: "Next page" }).click();
      await expect(team.getByRole("button", { name: "Restart team pages" })).toBeVisible();
    },
  );
  await team.getByRole("button", { name: "Restart team pages" }).click();
  await expect(team.getByRole("list", { name: "Employee list" }).getByRole("listitem")).toHaveCount(
    20,
  );
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await expect(page.getByRole("region", { name: "Team" })).toContainText(
    "No Employees in this project.",
  );
});
