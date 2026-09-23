import type { TaskKind } from "../contracts/common.ts";
import type { PipelineVersionView } from "../contracts/pipeline.ts";
import type { PrioritySchemeView } from "../contracts/priority-scheme.ts";
import { CreateTaskRequestSchema, type CreateTaskAttempt } from "../contracts/create-task.ts";
import { IdempotencyKeySchema } from "../contracts/task-command.ts";
import { activePriority } from "./priority-attempt.ts";
import type { TaskProperties } from "../contracts/properties.ts";

export type CreateTaskFields = {
  title: string;
  description: string;
  definitionOfDone: string;
  kind: TaskKind | "";
  priority: string;
  pipelineVersionId: string;
};
export function initialCreateTaskFields(scheme: PrioritySchemeView): CreateTaskFields {
  return {
    title: "",
    description: "",
    definitionOfDone: "",
    kind: "",
    priority: scheme.default_level_id,
    pipelineVersionId: "",
  };
}
export function createTaskInput(
  scheme: PrioritySchemeView,
  fields: CreateTaskFields,
  properties: TaskProperties = {},
) {
  return {
    project_id: scheme.project_id,
    expected_revision: scheme.project_revision,
    payload: {
      title: fields.title,
      description: fields.description,
      definition_of_done: fields.definitionOfDone === "" ? null : fields.definitionOfDone,
      kind: fields.kind,
      priority: fields.priority,
      pipeline_version_id: fields.pipelineVersionId,
      properties,
    },
  };
}
export function pipelineAllowsCreation(
  pipeline: PipelineVersionView | undefined,
  fields: CreateTaskFields,
) {
  return (
    pipeline !== undefined &&
    pipeline.id === fields.pipelineVersionId &&
    pipeline.deleted_at === null &&
    fields.kind !== "" &&
    pipeline.task_kinds.includes(fields.kind)
  );
}
export function createTaskAttempt(
  scheme: PrioritySchemeView,
  pipeline: PipelineVersionView,
  fields: CreateTaskFields,
  key: string = crypto.randomUUID(),
  properties: TaskProperties = {},
): CreateTaskAttempt {
  if (!activePriority(scheme, fields.priority) || !pipelineAllowsCreation(pipeline, fields))
    throw new Error("Draft creation is unavailable");
  const request = CreateTaskRequestSchema.parse(createTaskInput(scheme, fields, properties));
  return Object.freeze({ body: JSON.stringify(request), key: IdempotencyKeySchema.parse(key) });
}
