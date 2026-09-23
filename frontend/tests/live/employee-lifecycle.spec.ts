import { randomUUID } from "node:crypto";
import { test, expect, type Live } from "./fixtures";
import { holdRealResponse, loadProject } from "./task-helpers";
import { EmployeeProfileSchema } from "../../src/contracts/employee";
import { ProjectViewSchema } from "../../src/contracts/project";
import { EmployeeCommandReceiptSchema } from "../../src/contracts/create-employee";

async function createEmployee(live: Live) {
  const projectId = live.core.employee_creation_project.id;
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const project = ProjectViewSchema.parse(
    await (await fetch(`${live.origin}/api/projects/${projectId}`, { headers })).json(),
  );
  const name = `Managed ${randomUUID()}`;
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

async function openControls(live: Live, page: import("@playwright/test").Page, name: string) {
  await live.login(page);
  await loadProject(page, live.core.employee_creation_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page.getByRole("button", { name: `Open Employee ${name}` }).click();
  const profile = page.getByRole("region", { name: "Employee profile" });
  await profile.getByRole("button", { name: "Manage state" }).click();
  const controls = page.getByRole("region", { name: "Manage Employee state" });
  await expect(controls).toContainText(name);
  return { profile, controls };
}

async function canonical(live: Live, created: Awaited<ReturnType<typeof createEmployee>>) {
  return EmployeeProfileSchema.parse(
    await (
      await fetch(
        `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`,
        {
          headers: { Authorization: `Bearer ${created.token}` },
        },
      )
    ).json(),
  );
}

test("Employee can be disabled, re-enabled and permanently retired with optional audit reason", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  await page.setViewportSize({ width: 375, height: 812 });
  const { profile, controls } = await openControls(live, page, created.name);
  await controls.getByRole("button", { name: "Disable Employee" }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText(created.name);
  await expect(dialog.getByRole("heading")).toBeFocused();
  await expect(dialog).toContainText("Active Runs are not stopped");
  await dialog.getByLabel("Reason (optional)").fill("Capacity review");
  await dialog.getByRole("button", { name: "Confirm Disable Employee" }).click();
  await expect(controls).toHaveCount(0);
  await expect(profile).toContainText("disabled");
  expect((await canonical(live, created)).state).toBe("disabled");
  await profile.getByRole("button", { name: "Manage state" }).click();
  await controls.getByRole("button", { name: "Enable Employee" }).click();
  await dialog.getByRole("button", { name: "Confirm Enable Employee" }).click();
  await expect(controls).toHaveCount(0);
  expect((await canonical(live, created)).state).toBe("enabled");
  await profile.getByRole("button", { name: "Manage state" }).click();
  await controls.getByRole("button", { name: "Retire Employee" }).click();
  await expect(dialog).toContainText("Retirement is final");
  await dialog.getByLabel("Reason (optional)").fill("Position closed");
  await dialog.getByRole("button", { name: "Confirm Retire Employee" }).click();
  await expect(controls).toHaveCount(0);
  expect((await canonical(live, created)).state).toBe("retired");
  await expect(profile.getByRole("button", { name: "Edit Employee" })).toBeDisabled();
  await profile.getByRole("button", { name: "Manage state" }).click();
  await expect(controls).toContainText("Retirement is final");
  await expect(controls.getByRole("button", { name: "Enable Employee" })).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
});

test("unknown Employee state response replays identical bytes and key once", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  const { controls } = await openControls(live, page, created.name);
  const path = `${live.origin}/api/commands/disable_employee`;
  const attempts: { body: string | null; key: string | undefined }[] = [];
  let firstReceipt: unknown;
  let replayReceipt: unknown;
  await page.route(path, async (route) => {
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
    await controls.getByRole("button", { name: "Disable Employee" }).click();
    const dialog = page.getByRole("alertdialog");
    await dialog.getByLabel("Reason (optional)").fill("Manual hold");
    await dialog.getByRole("button", { name: "Confirm Disable Employee" }).click();
    await expect(controls.getByRole("button", { name: "Retry same request" })).toBeVisible();
  });
  await controls.getByRole("button", { name: "Retry same request" }).click();
  await expect(controls).toHaveCount(0);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  EmployeeCommandReceiptSchema.parse(firstReceipt);
  EmployeeCommandReceiptSchema.parse(replayReceipt);
  expect(replayReceipt).toEqual({ ...(firstReceipt as object), status: "replayed" });
  expect((await canonical(live, created)).revision).toBe(2);
});

test("accepted state change retries readback without sending another command", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  const { controls } = await openControls(live, page, created.name);
  let commands = 0;
  page.on("request", (request) => {
    if (request.url() === `${live.origin}/api/commands/disable_employee`) commands += 1;
  });
  const profilePath = `${live.origin}/api/projects/${created.projectId}/employees/${created.employeeId}`;
  await page.route(profilePath, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable", code: "unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await controls.getByRole("button", { name: "Disable Employee" }).click();
      await page
        .getByRole("alertdialog")
        .getByRole("button", { name: "Confirm Disable Employee" })
        .click();
      await expect(controls.getByRole("button", { name: "Retry profile read" })).toBeVisible();
    },
  );
  await expect(controls).toContainText("Receipt");
  await page.unroute(profilePath);
  await controls.getByRole("button", { name: "Retry profile read" }).click();
  await expect(controls).toHaveCount(0);
  expect(commands).toBe(1);
  expect((await canonical(live, created)).state).toBe("disabled");
});

test("Employee state conflict refreshes both revisions before another action", async ({
  live,
  page,
}) => {
  const created = await createEmployee(live);
  const { controls } = await openControls(live, page, created.name);
  const project = ProjectViewSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${created.projectId}`, {
        headers: { Authorization: `Bearer ${created.token}` },
      })
    ).json(),
  );
  const employee = await canonical(live, created);
  const body = {
    project_id: created.projectId,
    expected_revision: project.revision,
    payload: {
      employee_id: created.employeeId,
      expected_employee_revision: employee.revision,
      reason: "Concurrent owner change",
    },
  };
  const headers = {
    Authorization: `Bearer ${created.token}`,
    Origin: live.origin,
    "Content-Type": "application/json",
    "Idempotency-Key": randomUUID(),
  };
  const otherProject = ProjectViewSchema.parse(
    await (
      await fetch(`${live.origin}/api/projects/${live.core.empty_project.id}`, {
        headers: { Authorization: `Bearer ${created.token}` },
      })
    ).json(),
  );
  const foreign = await fetch(`${live.origin}/api/commands/disable_employee`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      ...body,
      project_id: otherProject.id,
      expected_revision: otherProject.revision,
    }),
  });
  expect(foreign.status).toBe(404);
  const applied = await fetch(`${live.origin}/api/commands/disable_employee`, {
    method: "POST",
    headers: { ...headers, "Idempotency-Key": randomUUID() },
    body: JSON.stringify(body),
  });
  expect(applied.status).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await controls.getByRole("button", { name: "Disable Employee" }).click();
      await page
        .getByRole("alertdialog")
        .getByRole("button", { name: "Confirm Disable Employee" })
        .click();
      await expect(
        controls.getByRole("button", { name: "Refresh Project and Employee" }),
      ).toBeVisible();
    },
  );
  await controls.getByRole("button", { name: "Refresh Project and Employee" }).click();
  await expect(controls.getByRole("button", { name: "Enable Employee" })).toBeVisible();
  await expect(controls.getByRole("button", { name: "Disable Employee" })).toHaveCount(0);
});

for (const destination of ["project", "logout"] as const) {
  test(`late Employee state receipt after ${destination} cannot restore old controls`, async ({
    live,
    page,
  }) => {
    const created = await createEmployee(live);
    const { controls } = await openControls(live, page, created.name);
    const held = await holdRealResponse(page, `${live.origin}/api/commands/disable_employee`);
    try {
      await controls.getByRole("button", { name: "Disable Employee" }).click();
      await page
        .getByRole("alertdialog")
        .getByRole("button", { name: "Confirm Disable Employee" })
        .click();
      await held.ready;
      held.assertCaptured();
      expect((await canonical(live, created)).state).toBe("disabled");
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
      await expect(controls).toHaveCount(0);
    } finally {
      await held.cleanup();
    }
  });
}
