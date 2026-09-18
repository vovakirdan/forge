import type { Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";
import { commandClient, draftForm } from "./draft-helpers";
import { loadProject } from "./task-helpers";

export const priorityCommandPath = "/api/commands/set_task_priority";
export const priorityEditor = (page: Page) =>
  page.getByRole("region", { name: "Change priority", exact: true });

export function priorityDraft(live: Live, index: number) {
  const draft = live.core.priority_commands_project.drafts[index];
  if (!draft) throw new Error("Missing isolated priority command fixture");
  return draft;
}

export async function openPriority(page: Page, live: Live, index: number) {
  await live.login(page);
  await loadProject(page, live.core.priority_commands_project.id);
  const draft = priorityDraft(live, index);
  await page.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Change priority", exact: true }).click();
  await expect(page.getByLabel("Task priority", { exact: true })).toBeEnabled();
  return draft;
}

export async function priorityClient(live: Live) {
  const client = await commandClient(live, {
    path: priorityCommandPath,
    projectId: live.core.priority_commands_project.id,
  });
  return {
    ...client,
    async priorityBaseline(id: string, priority: string) {
      const project = await client.project();
      const task = await client.task(id);
      return {
        project_id: project.id,
        expected_revision: project.revision,
        payload: { task_id: id, expected_task_revision: task.revision, priority },
      };
    },
  };
}
