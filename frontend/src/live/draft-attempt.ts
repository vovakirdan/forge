import { AmendDraftRequestSchema, IdempotencyKeySchema } from "../contracts/amend-draft.ts";
import type { ProjectView } from "../contracts/project.ts";
import type { TaskDetailView } from "../contracts/task.ts";

export type DraftFields = { title: string; description: string };
export type DraftBaseline = { project: ProjectView; task: TaskDetailView };
export type DraftAttempt = Readonly<{ body: string; key: string; taskId: string }>;

export function draftChanged(fields: DraftFields, baseline: DraftFields) {
  return fields.title !== baseline.title || fields.description !== baseline.description;
}

/** Refresh untouched fields while retaining only the user's edited-field intent. */
export function rebaseDraftFields(fields: DraftFields, before: DraftFields, after: DraftFields) {
  return {
    title: fields.title === before.title ? after.title : fields.title,
    description: fields.description === before.description ? after.description : fields.description,
  };
}

export function createDraftAttempt(
  baseline: DraftBaseline,
  fields: DraftFields,
  key: string = crypto.randomUUID(),
): DraftAttempt {
  if (baseline.task.lifecycle !== "draft") throw new Error("Task is not a draft");
  const request = AmendDraftRequestSchema.parse({
    project_id: baseline.project.id,
    expected_revision: baseline.project.revision,
    payload: {
      task_id: baseline.task.id,
      expected_task_revision: baseline.task.revision,
      patch: {
        ...(fields.title === baseline.task.title ? {} : { title: fields.title }),
        ...(fields.description === baseline.task.description
          ? {}
          : { description: fields.description }),
      },
    },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    taskId: request.payload.task_id,
  });
}
