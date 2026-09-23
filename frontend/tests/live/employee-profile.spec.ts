import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { EmployeeProfileSchema, EmployeeRunListResponseSchema } from "../../src/contracts/employee";

test("Employee profile and retained Run history are scoped canonical reads", async ({
  live,
  page,
}) => {
  const project = live.core.runs_project;
  const employeeId = project.employee_id;
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const path = `${live.origin}/api/projects/${project.id}/employees/${employeeId}`;
  const response = await fetch(path, { headers });
  expect(response.status).toBe(200);
  const profile = EmployeeProfileSchema.parse(await response.json());
  expect(profile.id).toBe(employeeId);
  expect(profile.project_id).toBe(project.id);
  expect(profile.stage_eligibility.mode).toBe("any");
  expect(Object.keys(profile).sort()).toEqual(
    [
      "created_at",
      "id",
      "max_concurrent_runs",
      "name",
      "project_id",
      "revision",
      "role",
      "stage_eligibility",
      "state",
      "updated_at",
    ].sort(),
  );

  const firstResponse = await fetch(`${path}/runs?limit=20`, { headers });
  expect(firstResponse.status).toBe(200);
  const first = EmployeeRunListResponseSchema.parse(await firstResponse.json());
  expect(first.items).toHaveLength(20);
  expect(first.next_cursor).toBeTruthy();
  const secondResponse = await fetch(`${path}/runs?limit=20&cursor=${first.next_cursor}`, {
    headers,
  });
  expect(secondResponse.status).toBe(200);
  const second = EmployeeRunListResponseSchema.parse(await secondResponse.json());
  expect(second.items).toHaveLength(3);
  expect(second.next_cursor).toBeUndefined();
  const all = [...first.items, ...second.items];
  expect(new Set(all.map((run) => run.id)).size).toBe(23);
  expect(all.every((run) => run.employee_id === employeeId)).toBe(true);
  const projectRuns = await fetch(`${live.origin}/api/projects/${project.id}/runs?limit=100`, {
    headers,
  });
  expect(projectRuns.status).toBe(200);
  const projectPage = EmployeeRunListResponseSchema.parse(await projectRuns.json());
  expect(all.map((run) => run.id)).toEqual(projectPage.items.map((run) => run.id));

  const foreign = await fetch(
    `${live.origin}/api/projects/${live.core.empty_project.id}/employees/${employeeId}`,
    { headers },
  );
  expect(foreign.status).toBe(404);
  const foreignRuns = await fetch(
    `${live.origin}/api/projects/${live.core.empty_project.id}/employees/${employeeId}/runs?limit=20`,
    { headers },
  );
  expect(foreignRuns.status).toBe(404);
  const stale = await fetch(`${path}/runs?limit=20&cursor=missing`, { headers });
  expect(stale.status).toBe(409);
  expect(await stale.json()).toMatchObject({ code: "cursor_invalid" });

  await page.setViewportSize({ width: 375, height: 812 });
  await live.login(page);
  await loadProject(page, project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  const open = page.getByRole("button", { name: "Open Employee Deterministic M0 fixture worker" });
  await open.focus();
  await page.keyboard.press("Enter");
  const profilePanel = page.getByRole("region", { name: "Employee profile" });
  await expect(profilePanel).toContainText(profile.name);
  const history = page.getByRole("region", { name: "Employee Run history" });
  await expect(
    history.getByRole("list", { name: "Employee Runs" }).getByRole("listitem"),
  ).toHaveCount(20);
  await history.getByRole("button", { name: "Next Run page" }).click();
  await expect(
    history.getByRole("list", { name: "Employee Runs" }).getByRole("listitem"),
  ).toHaveCount(3);
  const runButton = history.getByRole("button", { name: `Open Run ${second.items[0]?.id}` });
  await runButton.click();
  await expect(page.getByRole("region", { name: "Run details" })).toBeVisible();
  await page.getByRole("button", { name: "Close run" }).click();
  await expect(runButton).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await profilePanel.getByRole("button", { name: "Close profile" }).click();
  await expect(open).toBeFocused();
});

test("profile keeps canonical fields when history fails and empty history is explicit", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.team_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  const team = page.getByRole("region", { name: "Team" });
  const open = team.getByRole("button", { name: "Open Employee Team member 00" });
  await open.click();
  const profile = page.getByRole("region", { name: "Employee profile" });
  await expect(profile).toContainText("disabled");
  const history = page.getByRole("region", { name: "Employee Run history" });
  await expect(history).toContainText("No Runs for this Employee.");
  const url = `${live.origin}/api/projects/${live.core.team_project.id}/employees/${live.core.team_project.disabled_id}/runs?limit=20`;
  await page.route(url, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable", code: "unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await history.getByRole("button", { name: "Refresh history" }).click();
      await expect(history.getByRole("alert")).toContainText("Showing stale Run history.");
    },
  );
  await expect(profile).toContainText("Team member 00");
  await page.unroute(url);
  await history.getByRole("button", { name: "Refresh history" }).click();
  await expect(history).toContainText("No Runs for this Employee.");
});

test("Employee Run pagination recovers an invalid cursor", async ({ live, page }) => {
  await live.login(page);
  await loadProject(page, live.core.runs_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page.getByRole("button", { name: "Open Employee Deterministic M0 fixture worker" }).click();
  const history = page.getByRole("region", { name: "Employee Run history" });
  await expect(history.getByRole("button", { name: "Next Run page" })).toBeEnabled();
  await page.route("**/employees/*/runs?limit=20&cursor=*", (route) =>
    route.fulfill({ status: 409, json: { error: "Expired", code: "cursor_invalid" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/,
    async () => {
      await history.getByRole("button", { name: "Next Run page" }).click();
      await expect(history.getByRole("button", { name: "Restart Run history" })).toBeVisible();
    },
  );
  await history.getByRole("button", { name: "Restart Run history" }).click();
  await expect(
    history.getByRole("list", { name: "Employee Runs" }).getByRole("listitem"),
  ).toHaveCount(20);
});
