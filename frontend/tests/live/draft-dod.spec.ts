import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { commandClient, commandPath, draftForm } from "./draft-helpers";
import { createClient, fillCreate, openCreate, submitCreate } from "./create-command-helpers";
import { loadProject } from "./task-helpers";
import { approvalPanel, confirmApproval, consent } from "./approval-helpers";

test("new draft can be saved and separately approved through the complete browser flow", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  await fillCreate(page, live, `Create then approve ${randomUUID()}`);
  const receipt = await submitCreate(page);
  const client = await createClient(live);
  expect((await client.task(receipt.resource.id)).definition_of_done).toBeNull();
  await draftForm(page).getByRole("button", { name: "Edit draft", exact: true }).click();
  await page
    .getByLabel("Draft definition of done", { exact: true })
    .fill("Accepted written result");
  await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(draftForm(page)).toContainText("Saved.");
  expect((await client.task(receipt.resource.id)).lifecycle).toBe("draft");
  await page.getByRole("button", { name: "Close editor", exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Approve draft", exact: true }).click();
  await consent(page).check();
  await confirmApproval(page).click();
  await expect(approvalPanel(page)).toContainText("Authoritative data refreshed.");
  expect(await client.task(receipt.resource.id)).toMatchObject({
    lifecycle: "ready",
    pipeline_version_id: live.core.create_commands_project.older_version_id,
    definition_of_done: "Accepted written result",
  });
  expect((await client.runs()).items).toEqual([]);
  expect((await client.project()).execution_gate).toBe("stopped");
});

test("DoD can be set, replaced and cleared without approval or changing other fields", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  await fillCreate(page, live, `DoD edits ${randomUUID()}`);
  const created = await submitCreate(page);
  const client = await createClient(live);
  const original = await client.task(created.resource.id);
  let approvals = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/api/commands/approve_task")) approvals += 1;
  });
  for (const value of ["First acceptance 🛠", "Revised acceptance", ""]) {
    await draftForm(page).getByRole("button", { name: "Edit draft", exact: true }).click();
    await page.getByLabel("Draft definition of done", { exact: true }).fill(value);
    const sent = page.waitForRequest(`${live.origin}${commandPath}`);
    await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
    expect((await sent).postDataJSON().payload.patch).toEqual({
      definition_of_done: value || null,
    });
    await expect(draftForm(page)).toContainText("Saved.");
    const saved = await client.task(original.id);
    expect(saved.definition_of_done).toBe(value || null);
    expect(saved.lifecycle).toBe("draft");
    expect(saved.title).toBe(original.title);
    expect(saved.pipeline_version_id).toBe(original.pipeline_version_id);
    await page.getByRole("button", { name: "Close editor", exact: true }).click();
  }
  expect(approvals).toBe(0);
  expect((await client.runs()).items).toEqual([]);
});

test("DoD conflict preserves edited intent and refreshes untouched description", async ({
  live,
  page,
}) => {
  const creator = await createClient(live);
  const response = await creator.post(await creator.baseline(`DoD conflict ${randomUUID()}`));
  expect(response.status).toBe(200);
  const receipt = await response.json();
  const task = await creator.task(receipt.resource.id);
  await live.login(page);
  await loadProject(page, live.core.create_commands_project.id);
  await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Edit draft", exact: true }).click();
  await page.getByLabel("Draft definition of done", { exact: true }).fill("Keep my DoD");
  const editor = await commandClient(live, { projectId: live.core.create_commands_project.id });
  expect(
    (
      await editor.post(
        await editor.baseline(task.id, {
          description: "Concurrent description",
          definition_of_done: "Other DoD",
        }),
      )
    ).status,
  ).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
      await expect(
        page.getByRole("button", { name: "Refresh edit baseline", exact: true }),
      ).toBeVisible();
    },
  );
  await page.getByRole("button", { name: "Refresh edit baseline", exact: true }).click();
  await expect(page.getByLabel("Draft description", { exact: true })).toHaveValue(
    "Concurrent description",
  );
  await expect(page.getByLabel("Draft definition of done", { exact: true })).toHaveValue(
    "Keep my DoD",
  );
  await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(draftForm(page)).toContainText("Saved.");
  expect(await editor.task(task.id)).toMatchObject({
    definition_of_done: "Keep my DoD",
    description: "Concurrent description",
    lifecycle: "draft",
  });
  await page.getByText("Current saved values", { exact: true }).click();
  const saved = draftForm(page).getByRole("definition").filter({ hasText: "Keep my DoD" });
  await expect(saved).toBeVisible();
  await expect(draftForm(page)).not.toContainText("Other DoD");
});
