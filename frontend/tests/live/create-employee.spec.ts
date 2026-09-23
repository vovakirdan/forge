import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { ProjectViewSchema } from "../../src/contracts/project";
import { EmployeeCommandReceiptSchema } from "../../src/contracts/create-employee";
import { EmployeeListResponseSchema, EmployeeProfileSchema } from "../../src/contracts/employee";

const commandPath = "/api/commands/create_employee";

test("Team creates an Employee with a selected Pipeline stage and reads back Core state", async ({
  live,
  page,
}) => {
  const projectId = live.core.employee_creation_project.id;
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const beforeResponse = await fetch(`${live.origin}/api/projects/${projectId}`, { headers });
  const before = ProjectViewSchema.parse(await beforeResponse.json());
  await page.setViewportSize({ width: 375, height: 812 });
  await live.login(page);
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  const team = page.getByRole("region", { name: "Team" });
  await team.getByRole("button", { name: "Create Employee" }).click();
  const form = page.getByRole("region", { name: "Create Employee" });
  await expect(form.getByLabel("Employee name")).toBeFocused();
  await expect(form.getByRole("button", { name: "Create Employee" })).toBeDisabled();
  const name = `Selected stage ${randomUUID()}`;
  await form.getByLabel("Employee name").fill(name);
  await form.getByLabel("Role").fill("reviewer");
  await form.getByLabel("Only selected stages").check();
  await expect(form.getByRole("checkbox").first()).toBeVisible();
  await form.getByRole("checkbox").first().check();
  await expect(form.getByRole("button", { name: "Create Employee" })).toBeEnabled();
  await form.getByRole("button", { name: "Create Employee" }).click();
  const profile = page.getByRole("region", { name: "Employee profile" });
  await expect(profile).toContainText(name);
  const after = ProjectViewSchema.parse(
    await (await fetch(`${live.origin}/api/projects/${projectId}`, { headers })).json(),
  );
  expect(after.revision).toBe(before.revision + 1);
  const roster = EmployeeListResponseSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${projectId}/employees?limit=20`, { headers })
    ).json(),
  );
  const created = roster.items.find((employee) => employee.name === name);
  expect(created).toBeTruthy();
  const actualResponse = await fetch(
    `${live.origin}/api/projects/${projectId}/employees/${created?.id}`,
    { headers },
  );
  const actual = EmployeeProfileSchema.parse(await actualResponse.json());
  expect(actual).toMatchObject({
    id: created?.id,
    project_id: projectId,
    name,
    role: "reviewer",
    state: "enabled",
  });
  expect(actual.stage_eligibility.mode).toBe("only");
  if (actual.stage_eligibility.mode === "only") {
    expect(actual.stage_eligibility.stages).toHaveLength(1);
    expect(actual.stage_eligibility.stages[0]?.stage_id).toBe("work");
  }
  await profile.getByRole("button", { name: "Close profile" }).click();
  await expect(team.getByRole("list", { name: "Employee list" })).toContainText(name);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.reload();
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await expect(page.getByRole("region", { name: "Team" })).toContainText(name);
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await expect(page.getByRole("region", { name: "Team" })).not.toContainText(name);
});

test("unknown creation reuses identical bytes and key and resolves to one Employee", async ({
  live,
  page,
}) => {
  const projectId = live.core.employee_creation_project.id;
  await live.login(page);
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page
    .getByRole("region", { name: "Team" })
    .getByRole("button", { name: "Create Employee" })
    .click();
  const form = page.getByRole("region", { name: "Create Employee" });
  const name = `Replay employee ${randomUUID()}`;
  await form.getByLabel("Employee name").fill(name);
  await form.getByLabel("Role").fill("reviewer");
  await form.getByLabel("Any employee stage").check();
  const attempts: { body: string | null; key: string | undefined }[] = [];
  let firstReceipt: unknown;
  let replayReceipt: unknown;
  let failed = false;
  await page.route(`${live.origin}${commandPath}`, async (route) => {
    try {
      attempts.push({
        body: route.request().postData(),
        key: route.request().headers()["idempotency-key"],
      });
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected Core receipt");
      if (attempts.length === 1) {
        firstReceipt = await response.json();
        await route.abort("failed");
      } else {
        replayReceipt = await response.json();
        await route.fulfill({ response });
      }
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, async () => {
    await form.getByRole("button", { name: "Create Employee" }).click();
    await expect(form.getByRole("button", { name: "Retry same creation" })).toBeVisible();
  });
  await expect(form.getByLabel("Employee name")).toBeDisabled();
  await form.getByRole("button", { name: "Retry same creation" }).click();
  await expect(page.getByRole("region", { name: "Employee profile" })).toContainText(name);
  expect(failed).toBe(false);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  EmployeeCommandReceiptSchema.parse(firstReceipt);
  EmployeeCommandReceiptSchema.parse(replayReceipt);
  expect(replayReceipt).toEqual({ ...(firstReceipt as object), status: "replayed" });
  await page.unrouteAll({ behavior: "wait" });
  const token = await live.bearer();
  const employees = EmployeeListResponseSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${projectId}/employees?limit=20`, {
        headers: { Authorization: `Bearer ${token}` },
      })
    ).json(),
  );
  expect(employees.items.filter((employee) => employee.name === name)).toHaveLength(1);
});

test("accepted creation retries only profile readback after a read failure", async ({
  live,
  page,
}) => {
  const projectId = live.core.employee_creation_project.id;
  await live.login(page);
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page
    .getByRole("region", { name: "Team" })
    .getByRole("button", { name: "Create Employee" })
    .click();
  const form = page.getByRole("region", { name: "Create Employee" });
  const name = `Readback employee ${randomUUID()}`;
  await form.getByLabel("Employee name").fill(name);
  await form.getByLabel("Role").fill("reviewer");
  await form.getByLabel("Any employee stage").check();
  let commands = 0;
  page.on("request", (request) => {
    if (request.url() === `${live.origin}${commandPath}`) commands += 1;
  });
  const readPath = `${live.origin}/api/projects/${projectId}/employees/*`;
  await page.route(readPath, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable", code: "unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await form.getByRole("button", { name: "Create Employee" }).click();
      await expect(form.getByRole("button", { name: "Retry profile readback" })).toBeVisible();
    },
  );
  await expect(form).toContainText("Receipt");
  await page.unroute(readPath);
  await form.getByRole("button", { name: "Retry profile readback" }).click();
  await expect(page.getByRole("region", { name: "Employee profile" })).toContainText(name);
  expect(commands).toBe(1);
});

test("creation refuses stale Project revision and hostile requests before a new attempt", async ({
  live,
  page,
}) => {
  const projectId = live.core.employee_creation_project.id;
  const token = await live.bearer();
  const post = (body: unknown, key = randomUUID(), extra: Record<string, string> = {}) =>
    fetch(`${live.origin}${commandPath}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        Origin: live.origin,
        "Content-Type": "application/json",
        "Idempotency-Key": key,
        ...extra,
      },
      body: JSON.stringify(body),
    });
  const before = ProjectViewSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${projectId}`, {
        headers: { Authorization: `Bearer ${token}` },
      })
    ).json(),
  );
  const payload = {
    name: `Concurrent ${randomUUID()}`,
    role: "reviewer",
    stage_eligibility: { mode: "any" },
  };
  const request = { project_id: projectId, expected_revision: before.revision, payload };
  expect(
    (
      await fetch(`${live.origin}${commandPath}`, {
        method: "GET",
        headers: { Authorization: `Bearer ${token}` },
      })
    ).status,
  ).toBe(404);
  expect(
    (
      await fetch(`${live.origin}${commandPath}?actor=owner`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, Origin: live.origin },
      })
    ).status,
  ).toBe(404);
  expect((await post(request, randomUUID(), { Origin: "https://foreign.invalid" })).status).toBe(
    403,
  );
  expect((await post({ ...request, payload: { ...payload, token: "secret" } })).status).toBe(400);
  expect(
    (
      await post({
        ...request,
        payload: {
          ...payload,
          stage_eligibility: {
            mode: "only",
            stages: [
              {
                pipeline_version_id: live.core.create_commands_project.foreign_version_id,
                stage_id: "work",
              },
            ],
          },
        },
      })
    ).status,
  ).toBe(400);
  await live.login(page);
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page
    .getByRole("region", { name: "Team" })
    .getByRole("button", { name: "Create Employee" })
    .click();
  const form = page.getByRole("region", { name: "Create Employee" });
  const name = `After conflict ${randomUUID()}`;
  await form.getByLabel("Employee name").fill(name);
  await form.getByLabel("Role").fill("reviewer");
  await form.getByLabel("Any employee stage").check();
  expect((await post(request)).status).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await form.getByRole("button", { name: "Create Employee" }).click();
      await expect(form.getByRole("button", { name: "Refresh creation baseline" })).toBeVisible();
    },
  );
  await expect(form.getByLabel("Employee name")).toHaveValue(name);
  await form.getByRole("button", { name: "Refresh creation baseline" }).click();
  await expect(form.getByRole("button", { name: "Create Employee" })).toBeEnabled();
  await form.getByRole("button", { name: "Create Employee" }).click();
  await expect(page.getByRole("region", { name: "Employee profile" })).toContainText(name);
});
