import type { Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";
import { commandClient, draftForm } from "./draft-helpers";
import { loadProject } from "./task-helpers";

export const cancellationPath = "/api/commands/cancel_task";
export const cancellationPanel = (page: Page) =>
  page.getByRole("region", { name: "Cancel task", exact: true });
export const confirmCancellation = (page: Page) =>
  cancellationPanel(page).getByRole("button", { name: "Confirm cancellation", exact: true });
export const reason = (page: Page) =>
  cancellationPanel(page).getByRole("combobox", { name: "Cancellation reason", exact: true });
export const note = (page: Page) =>
  cancellationPanel(page).getByRole("textbox", {
    name: "Cancellation note (optional)",
    exact: true,
  });
export const consent = (page: Page) =>
  page.getByRole("checkbox", {
    name: "I understand cancellation requests Run shutdown",
    exact: true,
  });

export async function cancellationClient(live: Live) {
  const client = await commandClient(live, {
    path: cancellationPath,
    projectId: live.core.cancellation_commands_project.id,
  });
  return {
    ...client,
    async cancellation(id: string, key = "unspecified", note?: string) {
      const project = await client.project();
      const task = await client.task(id);
      return {
        project_id: project.id,
        expected_revision: project.revision,
        payload: {
          task_id: id,
          expected_task_revision: task.revision,
          cancellation_reason_key: key,
          ...(note === undefined ? {} : { note }),
        },
      };
    },
  };
}

export function cancellationDraft(live: Live, index: number) {
  const task = live.core.cancellation_commands_project.drafts[index];
  if (!task) throw new Error("Missing isolated cancellation draft");
  return task;
}

export async function openCancellation(page: Page, live: Live, id: string) {
  const task = await (await cancellationClient(live)).task(id);
  await live.login(page);
  await loadProject(page, live.core.cancellation_commands_project.id);
  await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Cancel task", exact: true }).click();
  await expect(confirmCancellation(page)).toBeVisible();
  return task;
}

export async function selectCancellation(page: Page) {
  await reason(page).selectOption("unspecified");
  await consent(page).check();
}
