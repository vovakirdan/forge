import type { Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";
import { commandClient, draftForm } from "./draft-helpers";
import { loadProject } from "./task-helpers";

export const approvalPath = "/api/commands/approve_task";
export const approvalPanel = (page: Page) =>
  page.getByRole("region", { name: "Approve draft", exact: true });
export const confirmApproval = (page: Page) =>
  approvalPanel(page).getByRole("button", { name: "Confirm approval", exact: true });
export const consent = (page: Page) =>
  page.getByRole("checkbox", { name: "I understand approval permits execution", exact: true });

export async function approvalClient(live: Live) {
  const client = await commandClient(live, {
    path: approvalPath,
    projectId: live.core.approval_commands_project.id,
  });
  return {
    ...client,
    async approval(id: string) {
      const project = await client.project();
      const task = await client.task(id);
      return {
        project_id: project.id,
        expected_revision: project.revision,
        payload: { task_id: id, expected_task_revision: task.revision },
      };
    },
  };
}

export async function openApproval(page: Page, live: Live, id: string) {
  const task = await (await approvalClient(live)).task(id);
  await live.login(page);
  await loadProject(page, live.core.approval_commands_project.id);
  await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Approve draft", exact: true }).click();
  await expect(confirmApproval(page)).toBeVisible();
  return task;
}

export function approvalDraft(live: Live, index: number) {
  const task = live.core.approval_commands_project.drafts[index];
  if (!task) throw new Error("Missing isolated approval draft");
  return task;
}
