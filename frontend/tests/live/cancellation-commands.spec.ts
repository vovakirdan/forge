import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { TaskCommandReceiptSchema } from "../../src/contracts/task-command";
import { commandClient, draftForm } from "./draft-helpers";
import {
  cancellationClient,
  cancellationDraft,
  cancellationPanel,
  confirmCancellation,
  consent,
  note,
  openCancellation,
  reason,
  selectCancellation,
} from "./cancellation-helpers";

for (const lifecycle of ["ready", "waiting"] as const) {
  test(`cancellation closes a real ${lifecycle} Task without starting a Run`, async ({
    live,
    page,
  }) => {
    const id =
      lifecycle === "ready"
        ? live.core.cancellation_commands_project.ready_id
        : live.core.cancellation_commands_project.waiting_id;
    const before = await openCancellation(page, live, id);
    expect(before.lifecycle).toBe(lifecycle);
    await selectCancellation(page);
    await confirmCancellation(page).click();
    await expect(cancellationPanel(page)).toContainText("Authoritative data refreshed.");
    const client = await cancellationClient(live);
    expect((await client.task(id)).lifecycle).toBe("cancelled");
    expect((await client.runs()).items).toEqual([]);
    expect((await client.project()).execution_gate).toBe("stopped");
  });
}

test("cancellation requires an explicit reason and consent then displays canonical metadata", async ({
  live,
  page,
}) => {
  const task = await openCancellation(page, live, cancellationDraft(live, 0).id);
  const client = await cancellationClient(live);
  await expect(reason(page)).toHaveValue("");
  await expect(confirmCancellation(page)).toBeDisabled();
  await expect(consent(page)).toBeDisabled();
  await reason(page).selectOption("unspecified");
  await note(page).fill("Superseded by an owner decision");
  await consent(page).check();
  await confirmCancellation(page).click();
  await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
  await expect(cancellationPanel(page)).toContainText("Authoritative data refreshed.");
  const after = await client.task(task.id);
  expect(after.lifecycle).toBe("cancelled");
  expect(after.cancellation).toMatchObject({
    reason_id: "unspecified",
    note: "Superseded by an owner decision",
  });
  expect(after.cancellation?.cancelled_at).toBeTruthy();
  expect(after.cancellation?.cancelled_by.id).toBeTruthy();
  await page.getByRole("button", { name: "Close cancellation", exact: true }).click();
  await expect(draftForm(page)).toContainText("Unspecified");
  await expect(draftForm(page)).toContainText("Superseded by an owner decision");
  await expect(draftForm(page)).toContainText(
    after.cancellation?.cancelled_by.id ?? "missing actor",
  );
  await expect(
    draftForm(page).getByRole("button", { name: "Cancel task", exact: true }),
  ).toHaveCount(0);
  expect((await client.runs()).items).toEqual([]);
});

test("cancellation conflict preserves input but refreshed baseline requires consent again", async ({
  live,
  page,
}) => {
  const task = await openCancellation(page, live, cancellationDraft(live, 1).id);
  await selectCancellation(page);
  await note(page).fill("Keep this decision context");
  await consent(page).check();
  const editor = await commandClient(live, {
    projectId: live.core.cancellation_commands_project.id,
  });
  expect(
    (await editor.post(await editor.baseline(task.id, { title: "Concurrent owner edit" }))).status,
  ).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await confirmCancellation(page).click();
      await expect(
        page.getByRole("button", { name: "Refresh cancellation baseline", exact: true }),
      ).toBeVisible();
    },
  );
  expect((await editor.task(task.id)).lifecycle).toBe("draft");
  await page.getByRole("button", { name: "Refresh cancellation baseline", exact: true }).click();
  await expect(cancellationPanel(page)).toContainText("Concurrent owner edit");
  await expect(note(page)).toHaveValue("Keep this decision context");
  await expect(reason(page)).toHaveValue("unspecified");
  await expect(consent(page)).not.toBeChecked();
  await expect(confirmCancellation(page)).toBeDisabled();
  await consent(page).check();
  await confirmCancellation(page).click();
  await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
});

test("cancellation boundary rejects forged fields, reasons and scope; replay has one effect", async ({
  live,
}) => {
  const client = await cancellationClient(live);
  const task = cancellationDraft(live, 2);
  const body = await client.cancellation(task.id);
  for (const headers of [
    { Origin: "null" },
    { Origin: "https://untrusted.invalid" },
    { Host: "untrusted.invalid" },
    { Authorization: "Bearer invalid" },
  ])
    expect([401, 403]).toContain((await client.post(body, randomUUID(), headers)).status);
  for (const bad of [
    { ...body, actor: "human" },
    { ...body, payload: { ...body.payload, lifecycle: "cancelled" } },
    { ...body, payload: { ...body.payload, task_id: live.core.second_project.task_id } },
    {
      ...body,
      payload: { ...body.payload, expected_task_revision: body.payload.expected_task_revision + 1 },
    },
  ])
    expect((await client.post(bad)).status).toBe(400);
  const invalidReason = await client.post({
    ...body,
    payload: { ...body.payload, cancellation_reason_key: "unknown" },
  });
  expect(invalidReason.status).toBe(422);
  expect((await invalidReason.json()).code).toBe("validation_failed");
  expect((await client.project()).revision).toBe(body.expected_revision);
  const key = randomUUID();
  const response = await client.post(body, key);
  expect(response.status).toBe(200);
  const receipt = TaskCommandReceiptSchema.parse(await response.json());
  expect(TaskCommandReceiptSchema.parse(await (await client.post(body, key)).json())).toEqual({
    ...receipt,
    status: "replayed",
  });
  expect((await client.project()).revision).toBe(receipt.project_revision);
  expect((await client.task(task.id)).revision).toBe(body.payload.expected_task_revision + 1);
  expect((await client.post(body)).status).toBe(409);
  expect((await client.post(await client.cancellation(task.id))).status).toBe(422);
});

test("cancellation keyboard confirmation works at narrow width with optional note omitted", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  const task = await openCancellation(page, live, cancellationDraft(live, 3).id);
  await expect(cancellationPanel(page)).toContainText(/physical|physically/i);
  await reason(page).selectOption("unspecified");
  await consent(page).focus();
  await page.keyboard.press("Space");
  await expect(confirmCancellation(page)).toBeEnabled();
  const box = await confirmCancellation(page).boundingBox();
  expect(box).not.toBeNull();
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(375);
  await confirmCancellation(page).focus();
  await page.keyboard.press("Enter");
  await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
  const after = await (await cancellationClient(live)).task(task.id);
  expect(after.cancellation?.note).toBeNull();
});
