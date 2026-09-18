import { randomUUID } from "node:crypto";
import { request as httpRequest } from "node:http";
import type { Page } from "@playwright/test";
import { ProjectViewSchema } from "../../src/contracts/project";
import { TaskDetailViewSchema, TaskListResponseSchema } from "../../src/contracts/task";
import { RunListResponseSchema } from "../../src/contracts/run";
import { expect, type Live } from "./fixtures";
import { loadProject } from "./task-helpers";

export const commandPath = "/api/commands/amend_draft";
export const draftForm = (page: Page) =>
  page.getByRole("region", { name: "Task detail", exact: true });

export function fixtureDraft(live: Live, index: number) {
  const draft = live.core.commands_project.drafts[index];
  if (!draft) throw new Error("Missing isolated draft fixture");
  return draft;
}

export async function openDraft(page: Page, live: Live, index: number) {
  await live.login(page);
  await loadProject(page, live.core.commands_project.id);
  const draft = fixtureDraft(live, index);
  await page.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await draftForm(page).getByRole("button", { name: "Edit draft", exact: true }).click();
  await expect(page.getByLabel("Draft title", { exact: true })).toBeEditable();
  return draft;
}

export type DraftRequest = {
  project_id: string;
  expected_revision: number;
  payload: {
    task_id: string;
    expected_task_revision: number;
    patch: { title?: string; description?: string };
  };
};

export async function commandClient(
  live: Live,
  options: {
    path?: typeof commandPath | "/api/commands/set_task_priority" | "/api/commands/create_task";
    projectId?: string;
  } = {},
) {
  const token = await live.bearer();
  const projectId = options.projectId ?? live.core.commands_project.id;
  async function get(path: string) {
    try {
      const response = await fetch(`${live.origin}${path}`, {
        headers: { Authorization: `Bearer ${token}` },
        signal: AbortSignal.timeout(6000),
      });
      if (!response.ok) throw new Error("read failed");
      return (await response.json()) as unknown;
    } catch {
      throw new Error("Command acceptance read failed; raw diagnostics withheld");
    }
  }
  const project = async () => ProjectViewSchema.parse(await get(`/api/projects/${projectId}`));
  const task = async (id: string) =>
    TaskDetailViewSchema.parse(await get(`/api/projects/${projectId}/tasks/${id}`));
  return {
    project,
    task,
    async tasks() {
      return TaskListResponseSchema.parse(await get(`/api/projects/${projectId}/tasks`));
    },
    async runs() {
      return RunListResponseSchema.parse(await get(`/api/projects/${projectId}/runs`));
    },
    async baseline(id: string, patch: DraftRequest["payload"]["patch"]): Promise<DraftRequest> {
      const currentProject = await project();
      const currentTask = await task(id);
      return {
        project_id: projectId,
        expected_revision: currentProject.revision,
        payload: { task_id: id, expected_task_revision: currentTask.revision, patch },
      };
    },
    async post(body: unknown, key: string = randomUUID(), headers: Record<string, string> = {}) {
      // Node fetch replaces a supplied Host with the URL authority. Use HTTP
      // directly so the hostile-Host probe really sends the intended header.
      const serialized = JSON.stringify(body);
      return new Promise<Response>((resolve, reject) => {
        const fail = () =>
          reject(new Error("Command acceptance POST failed; raw diagnostics withheld"));
        const request = httpRequest(
          `${live.origin}${options.path ?? commandPath}`,
          {
            method: "POST",
            signal: AbortSignal.timeout(6000),
            headers: {
              Authorization: `Bearer ${token}`,
              Origin: live.origin,
              "Content-Type": "application/json",
              "Content-Length": Buffer.byteLength(serialized),
              "Idempotency-Key": key,
              ...headers,
            },
          },
          (response) => {
            let bytes = 0;
            const chunks: Buffer[] = [];
            response.on("data", (chunk: Buffer) => {
              bytes += chunk.length;
              if (bytes > 64 * 1024) {
                response.destroy();
                fail();
              } else chunks.push(chunk);
            });
            response.on("error", fail);
            response.on("end", () =>
              resolve(new Response(Buffer.concat(chunks), { status: response.statusCode ?? 500 })),
            );
          },
        );
        request.on("error", fail);
        request.end(serialized);
      });
    },
  };
}
