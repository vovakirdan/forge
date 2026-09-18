import { ApproveTaskRequestSchema } from "../contracts/approve-task.ts";
import { DraftDefinitionOfDoneSchema } from "../contracts/amend-draft.ts";
import { IdempotencyKeySchema, type TaskCommandAttempt } from "../contracts/task-command.ts";
import type { DraftBaseline } from "./draft-attempt.ts";
import type { PipelineVersionView } from "../contracts/pipeline.ts";

export type ApprovalBaseline = DraftBaseline & { pipeline: PipelineVersionView };
export function hasApprovalDoD(baseline: ApprovalBaseline) {
  return DraftDefinitionOfDoneSchema.safeParse(baseline.task.definition_of_done).success;
}
export function createApprovalAttempt(
  baseline: ApprovalBaseline,
  key = crypto.randomUUID(),
): TaskCommandAttempt {
  if (baseline.task.lifecycle !== "draft" || !hasApprovalDoD(baseline))
    throw new Error("Draft with saved DoD required");
  if (baseline.pipeline.id !== baseline.task.pipeline_version_id)
    throw new Error("Pinned Pipeline mismatch");
  const request = ApproveTaskRequestSchema.parse({
    project_id: baseline.project.id,
    expected_revision: baseline.project.revision,
    payload: { task_id: baseline.task.id, expected_task_revision: baseline.task.revision },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    taskId: request.payload.task_id,
  });
}
