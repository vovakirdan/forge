import { randomUUID } from "node:crypto";
import type { Page } from "@playwright/test";
import { test, expect, type Live } from "./fixtures";
import { holdRealResponse, loadProject } from "./task-helpers";
import { ProjectViewSchema } from "../../src/contracts/project";
import { EmployeeProfileSchema } from "../../src/contracts/employee";
import { EmployeeCommandReceiptSchema } from "../../src/contracts/create-employee";

const commandPath = "/api/commands/amend_employee";

async function createEmployee(live: Live) {
  const projectId = live.core.employee_creation_project.id;
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const project = ProjectViewSchema.parse(
    await (await fetch(`${live.origin}/api/projects/${projectId}`, { headers })).json(),
  );
  const name = `Editable ${randomUUID()}`;
  const response = await fetch(`${live.origin}/api/commands/create_employee`, {
    method: "POST",
    headers: {
      ...headers,
      Origin: live.origin,
      "Content-Type": "application/json",
      "Idempotency-Key": randomUUID(),
    },
    body: JSON.stringify({
      project_id: projectId,
      expected_revision: project.revision,
      payload: { name, role: "reviewer", stage_eligibility: { mode: "any" } },
    }),
  });
  expect(response.status).toBe(200);
  const receipt = EmployeeCommandReceiptSchema.parse(await response.json());
  return { projectId, token, employeeId: receipt.resource.id, name };
}

async function openEditor(page: Page, live: Live, employeeId: string, name: string) {
  await live.login(page);
  await loadProject(page, live.core.employee_creation_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page.getByRole("button", { name: `Open Employee ${name}` }).click();
  const profile = page.getByRole("region", { name: "Employee profile" });
  await profile.getByRole("button", { name: "Edit Employee" }).click();
  const editor = page.getByRole("region", { name: "Edit Employee" });
  await expect(editor.getByLabel("Employee name")).toBeFocused();
  await expect(profile).toContainText(employeeId);
  return { profile, editor };
}

test("profile amends all catalog fields and reads canonical Employee at narrow width", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  await page.setViewportSize({ width: 375, height: 812 });
  const { profile, editor } = await openEditor(page, live, created.employeeId, created.name);
  await expect(editor.getByRole("button", { name: "Save Employee" })).toBeDisabled();
  const updatedName = `Renamed ${randomUUID()}`;
  await editor.getByLabel("Employee name").fill(updatedName);
  await editor.getByLabel("Role").fill("analyst");
  await editor.getByLabel("Capacity (concurrent Runs)").fill("3");
  await editor.getByLabel("Only selected stages").check();
  await editor.getByRole("checkbox").first().check();
  await editor.getByRole("button", { name: "Save Employee" }).click();
  await expect(editor).toHaveCount(0);
  await expect(profile).toContainText(updatedName);
  await expect(profile).toContainText("3 concurrent Runs");
  const actual = EmployeeProfileSchema.parse(
    await (
      await fetch(
        `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`,
        {
          headers: { Authorization: `Bearer ${created.token}` },
        },
      )
    ).json(),
  );
  expect(actual).toMatchObject({
    name: updatedName,
    role: "analyst",
    max_concurrent_runs: 3,
    revision: 2,
  });
  expect(actual.stage_eligibility).toMatchObject({ mode: "only", stages: [{ stage_id: "work" }] });
  await expect(page.getByRole("region", { name: "Team" })).toContainText(updatedName);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
});

test("stale Employee amendment preserves draft until explicit baseline refresh", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  const { editor } = await openEditor(page, live, created.employeeId, created.name);
  const wanted = `Wanted ${randomUUID()}`;
  await editor.getByLabel("Employee name").fill(wanted);
  await expect(editor.getByRole("button", { name: "Save Employee" })).toBeEnabled();
  const project = ProjectViewSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${created.projectId}`, {
        headers: { Authorization: `Bearer ${created.token}` },
      })
    ).json(),
  );
  const post = (payload: object, projectRevision = project.revision) =>
    fetch(`${live.origin}${commandPath}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${created.token}`,
        Origin: live.origin,
        "Content-Type": "application/json",
        "Idempotency-Key": randomUUID(),
      },
      body: JSON.stringify({
        project_id: created.projectId,
        expected_revision: projectRevision,
        payload,
      }),
    });
  const base = { employee_id: created.employeeId, expected_employee_revision: 1 };
  expect((await post({ ...base, patch: { state: "disabled" } })).status).toBe(400);
  expect((await post({ ...base, patch: {} })).status).toBe(400);
  expect((await post({ ...base, patch: { max_concurrent_runs: 2 }, token: "secret" })).status).toBe(
    400,
  );
  expect(
    (
      await post({
        ...base,
        patch: {
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
  const foreignProject = ProjectViewSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${live.core.empty_project.id}`, {
        headers: { Authorization: `Bearer ${created.token}` },
      })
    ).json(),
  );
  expect(
    (
      await fetch(`${live.origin}${commandPath}`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${created.token}`,
          Origin: live.origin,
          "Content-Type": "application/json",
          "Idempotency-Key": randomUUID(),
        },
        body: JSON.stringify({
          project_id: live.core.empty_project.id,
          expected_revision: foreignProject.revision,
          payload: { ...base, patch: { name: "Foreign" } },
        }),
      })
    ).status,
  ).toBe(404);
  expect((await post({ ...base, patch: { name: "Concurrent" } })).status).toBe(200);
  expect(
    (await post({ ...base, patch: { name: "Stale Employee" } }, project.revision + 1)).status,
  ).toBe(409);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await editor.getByRole("button", { name: "Save Employee" }).click();
      await expect(editor.getByRole("button", { name: "Refresh edit baselines" })).toBeVisible();
    },
  );
  await expect(editor.getByLabel("Employee name")).toHaveValue(wanted);
  await editor.getByRole("button", { name: "Refresh edit baselines" }).click();
  await expect(editor.getByRole("button", { name: "Save Employee" })).toBeEnabled();
  await editor.getByRole("button", { name: "Save Employee" }).click();
  await expect(page.getByRole("region", { name: "Employee profile" })).toContainText(wanted);
});

test("lost amendment response replays exact bytes and key with one Employee revision", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  const { editor } = await openEditor(page, live, created.employeeId, created.name);
  const name = `Replay ${randomUUID()}`;
  await editor.getByLabel("Employee name").fill(name);
  const attempts: { body: string | null; key: string | undefined }[] = [];
  let firstReceipt: unknown;
  let replayReceipt: unknown;
  await page.route(`${live.origin}${commandPath}`, async (route) => {
    attempts.push({
      body: route.request().postData(),
      key: route.request().headers()["idempotency-key"],
    });
    const response = await route.fetch({ timeout: 10_000 });
    expect(response.status()).toBe(200);
    if (attempts.length === 1) {
      firstReceipt = await response.json();
      await route.abort("failed");
    } else {
      replayReceipt = await response.json();
      await route.fulfill({ response });
    }
  });
  await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, async () => {
    await editor.getByRole("button", { name: "Save Employee" }).click();
    await expect(editor.getByRole("button", { name: "Retry same save" })).toBeVisible();
  });
  await editor.getByRole("button", { name: "Retry same save" }).click();
  await expect(editor).toHaveCount(0);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  EmployeeCommandReceiptSchema.parse(firstReceipt);
  EmployeeCommandReceiptSchema.parse(replayReceipt);
  expect(replayReceipt).toEqual({ ...(firstReceipt as object), status: "replayed" });
  await page.unrouteAll({ behavior: "wait" });
  const actual = EmployeeProfileSchema.parse(
    await (
      await fetch(
        `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`,
        { headers: { Authorization: `Bearer ${created.token}` } },
      )
    ).json(),
  );
  expect(actual.revision).toBe(2);
  expect(actual.name).toBe(name);
});

test("accepted amendment retries only failed profile readback", async ({ live, page }) => {
  const created = await createEmployee(live);
  const { editor } = await openEditor(page, live, created.employeeId, created.name);
  const name = `Readback ${randomUUID()}`;
  await editor.getByLabel("Employee name").fill(name);
  let commands = 0;
  page.on("request", (request) => {
    if (request.url() === `${live.origin}${commandPath}`) commands += 1;
  });
  const profilePath = `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`;
  await page.route(profilePath, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable", code: "unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await editor.getByRole("button", { name: "Save Employee" }).click();
      await expect(editor.getByRole("button", { name: "Retry profile readback" })).toBeVisible();
    },
  );
  await expect(editor).toContainText("Receipt");
  await page.unroute(profilePath);
  await editor.getByRole("button", { name: "Retry profile readback" }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Employee profile" })).toContainText(name);
  expect(commands).toBe(1);
});

test("unsaved Employee edit guards Project navigation and retired profiles stay read-only", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  const { editor } = await openEditor(page, live, created.employeeId, created.name);
  const draft = `Unsaved ${randomUUID()}`;
  await editor.getByLabel("Employee name").fill(draft);
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(editor.getByLabel("Employee name")).toHaveValue(draft);
  page.once("dialog", (dialog) => dialog.accept());
  await loadProject(page, live.core.team_project.id);
  await expect(editor).toHaveCount(0);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page.getByRole("button", { name: "Open Employee Team member 01" }).click();
  const retired = page.getByRole("region", { name: "Employee profile" });
  await expect(retired).toContainText("retired");
  await expect(retired.getByRole("button", { name: "Edit Employee" })).toBeDisabled();
  const actual = EmployeeProfileSchema.parse(
    await (
      await fetch(
        `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`,
        {
          headers: { Authorization: `Bearer ${created.token}` },
        },
      )
    ).json(),
  );
  expect(actual.name).toBe(created.name);
});

for (const destination of ["project", "logout"] as const) {
  test(`late amendment response after ${destination} cannot restore the old Employee UI`, async ({
    live,
    page,
  }) => {
    const created = await createEmployee(live);
    const { editor } = await openEditor(page, live, created.employeeId, created.name);
    const name = `Late ${randomUUID()}`;
    await editor.getByLabel("Employee name").fill(name);
    const held = await holdRealResponse(page, `${live.origin}${commandPath}`);
    try {
      await editor.getByRole("button", { name: "Save Employee" }).click();
      await held.ready;
      held.assertCaptured();
      const actual = EmployeeProfileSchema.parse(
        await (
          await fetch(
            `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`,
            {
              headers: { Authorization: `Bearer ${created.token}` },
            },
          )
        ).json(),
      );
      expect(actual.name).toBe(name);
      page.once("dialog", (dialog) => dialog.accept());
      if (destination === "project") {
        await loadProject(page, live.core.empty_project.id);
        await expect(page.getByRole("region", { name: "Project", exact: true })).toContainText(
          live.core.empty_project.id,
        );
      } else {
        await page.getByRole("button", { name: "Log out", exact: true }).click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      }
      await held.releaseAfterCancellation();
      await expect(editor).toHaveCount(0);
      await expect(page.getByRole("region", { name: "Employee profile" })).toHaveCount(0);
    } finally {
      await held.cleanup();
    }
  });
}
