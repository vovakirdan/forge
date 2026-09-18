import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { TaskCommandReceiptSchema } from "../../src/contracts/task-command";
import { commandClient, draftForm } from "./draft-helpers";
import {
  approvalClient,
  approvalDraft,
  approvalPanel,
  confirmApproval,
  consent,
  openApproval,
} from "./approval-helpers";

test("approval requires saved DoD and explicit consent, then reads canonical ready without opening gate", async ({
  live,
  page,
}) => {
  const id = live.core.approval_commands_project.without_dod_id;
  const before = await openApproval(page, live, id);
  const client = await approvalClient(live);
  await expect(confirmApproval(page)).toBeDisabled();
  await page.getByRole("button", { name: "Edit draft to add DoD", exact: true }).click();
  await page
    .getByLabel("Draft definition of done", { exact: true })
    .fill("Deliver reviewed evidence");
  await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(draftForm(page)).toContainText("Saved.");
  expect((await client.task(id)).lifecycle).toBe("draft");
  await page.getByRole("button", { name: "Close editor", exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Approve draft", exact: true }).click();
  await expect(approvalPanel(page)).toContainText("Deliver reviewed evidence");
  await expect(approvalPanel(page)).toContainText(before.pipeline_version_id);
  await expect(approvalPanel(page)).toContainText("stopped");
  await expect(confirmApproval(page)).toBeDisabled();
  await consent(page).check();
  await confirmApproval(page).click();
  await expect(approvalPanel(page)).toContainText("Approved.");
  await expect(approvalPanel(page)).toContainText("Authoritative data refreshed.");
  const after = await client.task(id);
  expect(after.lifecycle).toBe("ready");
  expect(after.pipeline_version_id).toBe(before.pipeline_version_id);
  expect((await client.project()).execution_gate).toBe("stopped");
  expect((await client.runs()).items).toEqual([]);
});

for (const branch of [
  "human_id",
  "external_id",
  "dependency_id",
  "deleted_pipeline_task_id",
] as const) {
  test(`approval displays actual ${branch} lifecycle rather than assuming ready`, async ({
    live,
    page,
  }) => {
    const task = await openApproval(page, live, live.core.approval_commands_project[branch]);
    const client = await approvalClient(live);
    await consent(page).check();
    await confirmApproval(page).click();
    await expect(approvalPanel(page)).toContainText("Approved.");
    await expect(approvalPanel(page)).toContainText("Authoritative data refreshed.");
    const after = await client.task(task.id);
    expect(after.lifecycle).toBe(branch === "deleted_pipeline_task_id" ? "ready" : "waiting");
    expect(after.pipeline_version_id).toBe(task.pipeline_version_id);
    await expect(draftForm(page)).toContainText(after.lifecycle);
    expect((await client.runs()).items).toEqual([]);
  });
}

test("approval conflict refreshes saved content and requires new consent", async ({
  live,
  page,
}) => {
  const task = await openApproval(page, live, approvalDraft(live, 0).id);
  await consent(page).check();
  const editor = await commandClient(live, { projectId: live.core.approval_commands_project.id });
  expect(
    (
      await editor.post(
        await editor.baseline(task.id, { definition_of_done: "Changed in another tab" }),
      )
    ).status,
  ).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await confirmApproval(page).click();
      await expect(
        page.getByRole("button", { name: "Refresh approval baseline", exact: true }),
      ).toBeVisible();
    },
  );
  expect((await editor.task(task.id)).lifecycle).toBe("draft");
  await page.getByRole("button", { name: "Refresh approval baseline", exact: true }).click();
  await expect(approvalPanel(page)).toContainText("Changed in another tab");
  await expect(consent(page)).not.toBeChecked();
  await expect(confirmApproval(page)).toBeDisabled();
  await consent(page).check();
  await confirmApproval(page).click();
  await expect(approvalPanel(page)).toContainText("Approved.");
  expect((await editor.task(task.id)).lifecycle).toBe("ready");
});

test("approval boundary rejects hostile fields and stale revisions; exact replay has one effect", async ({
  live,
}) => {
  const client = await approvalClient(live);
  const task = approvalDraft(live, 1);
  const body = await client.approval(task.id);
  for (const headers of [
    { Origin: "null" },
    { Origin: "https://untrusted.invalid" },
    { Host: "untrusted.invalid" },
    { Authorization: "Bearer invalid" },
  ])
    expect([401, 403]).toContain((await client.post(body, randomUUID(), headers)).status);
  for (const bad of [
    { ...body, actor: "human" },
    { ...body, payload: { ...body.payload, lifecycle: "ready" } },
    { ...body, payload: { ...body.payload, execution_gate: "open" } },
    { ...body, payload: { task_id: task.id } },
  ])
    expect((await client.post(bad)).status).toBe(400);
  expect((await client.project()).revision).toBe(body.expected_revision);
  const key = randomUUID();
  const response = await client.post(body, key);
  expect(response.status).toBe(200);
  const applied = TaskCommandReceiptSchema.parse(await response.json());
  const replay = await client.post(body, key);
  expect(TaskCommandReceiptSchema.parse(await replay.json())).toEqual({
    ...applied,
    status: "replayed",
  });
  expect((await client.project()).revision).toBe(body.expected_revision + 1);
  expect((await client.task(task.id)).revision).toBe(body.payload.expected_task_revision + 1);
  expect((await client.post(body)).status).toBe(409);
  const fresh = await client.approval(approvalDraft(live, 2).id);
  expect(
    (
      await client.post({
        ...fresh,
        payload: {
          ...fresh.payload,
          expected_task_revision: fresh.payload.expected_task_revision + 1,
        },
      })
    ).status,
  ).toBe(400);
  expect(
    (
      await client.post({
        ...fresh,
        payload: { ...fresh.payload, task_id: live.core.second_project.task_id },
      })
    ).status,
  ).toBe(400);
  expect((await client.runs()).items).toEqual([]);
});

test("approval supports narrow keyboard confirmation and explicitly warns about open gate", async ({
  live,
  page,
}) => {
  const projectId = live.core.approval_commands_project.id;
  const url = `${live.origin}/api/projects/${projectId}`;
  // Presentation-only open-gate injection: actual Core Project stays stopped.
  await page.route(url, async (route) => {
    const response = await route.fetch();
    const project = await response.json();
    await route.fulfill({ response, json: { ...project, execution_gate: "open" } });
  });
  await page.setViewportSize({ width: 375, height: 812 });
  const task = await openApproval(page, live, approvalDraft(live, 3).id);
  await expect(approvalPanel(page)).toContainText("open");
  await expect(approvalPanel(page)).toContainText(/may start/i);
  await consent(page).focus();
  await page.keyboard.press("Space");
  await expect(confirmApproval(page)).toBeEnabled();
  const box = await confirmApproval(page).boundingBox();
  expect(box).not.toBeNull();
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(375);
  await confirmApproval(page).focus();
  await page.keyboard.press("Enter");
  await expect(approvalPanel(page)).toContainText("Approved.");
  const client = await approvalClient(live);
  expect((await client.task(task.id)).lifecycle).toBe("ready");
  expect((await client.project()).execution_gate).toBe("stopped");
  expect((await client.runs()).items).toEqual([]);
});
