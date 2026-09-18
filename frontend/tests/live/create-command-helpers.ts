import type { Page } from "@playwright/test";
import { TaskCommandReceiptSchema } from "../../src/contracts/task-command";
import { expect, type Live } from "./fixtures";
import { commandClient, draftForm } from "./draft-helpers";
import { loadProject } from "./task-helpers";

export const createPath = "/api/commands/create_task";
export const createForm = (page: Page) =>
  page.getByRole("region", { name: "Create draft Task", exact: true });
export const createButton = (page: Page) =>
  createForm(page).getByRole("button", { name: "Create draft", exact: true });

export async function openCreate(page: Page, live: Live) {
  await live.login(page);
  await loadProject(page, live.core.create_commands_project.id);
  await page.getByRole("button", { name: "Create draft", exact: true }).click();
  await expect(page.getByLabel("Draft title", { exact: true })).toBeEditable();
}

export async function fillCreate(
  page: Page,
  live: Live,
  title: string,
  kind: "delivery" | "analysis" = "delivery",
) {
  await page.getByLabel("Draft title", { exact: true }).fill(title);
  await page.getByLabel("Task kind", { exact: true }).selectOption(kind);
  await createForm(page)
    .getByRole("button", {
      name: `Select Pipeline version ${live.core.create_commands_project.older_version_id}`,
      exact: true,
    })
    .click();
  await expect(createButton(page)).toBeEnabled();
}

export async function submitCreate(page: Page) {
  const url = `${new URL(page.url()).origin}${createPath}`;
  let captured: unknown;
  let failed = false;
  // Read before delivering: opening the new detail unmounts and aborts the
  // completed form request, so Chromium may discard its response body handle.
  await page.route(url, async (route) => {
    try {
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real receipt");
      captured = await response.json();
      await route.fulfill({ response });
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  try {
    await createButton(page).click();
    await expect(draftForm(page)).toBeVisible();
    expect(failed, "Real creation receipt was captured").toBe(false);
    return TaskCommandReceiptSchema.parse(captured);
  } finally {
    await page.unroute(url);
  }
}

export async function createClient(live: Live) {
  const client = await commandClient(live, {
    path: createPath,
    projectId: live.core.create_commands_project.id,
  });
  return {
    ...client,
    async baseline(title: string) {
      const project = await client.project();
      return {
        project_id: project.id,
        expected_revision: project.revision,
        payload: {
          title,
          description: "",
          definition_of_done: null,
          kind: "delivery",
          priority: "normal",
          pipeline_version_id: live.core.create_commands_project.older_version_id,
          properties: {},
        },
      };
    },
  };
}
