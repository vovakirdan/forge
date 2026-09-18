import { CancelTaskRequestSchema } from "../contracts/cancel-task.ts";
import type { CancellationReasonsView } from "../contracts/cancellation-reasons.ts";
import type { LifecycleStatus } from "../contracts/common.ts";
import { IdempotencyKeySchema, type TaskCommandAttempt } from "../contracts/task-command.ts";
import type { DraftBaseline } from "./draft-attempt.ts";

export type CancellationBaseline = DraftBaseline & { catalog: CancellationReasonsView };
export function canCancelTask(lifecycle: LifecycleStatus) {
  return lifecycle !== "done" && lifecycle !== "cancelled";
}
export function createCancellationAttempt(
  baseline: CancellationBaseline,
  reason: string,
  note: string,
  key = crypto.randomUUID(),
): TaskCommandAttempt {
  if (!canCancelTask(baseline.task.lifecycle)) throw new Error("Task is terminal");
  if (
    baseline.catalog.project_id !== baseline.project.id ||
    baseline.catalog.project_revision !== baseline.project.revision
  )
    throw new Error("Cancellation catalog baseline mismatch");
  if (!baseline.catalog.reasons.some((entry) => entry.id === reason && !entry.retired))
    throw new Error("Active cancellation reason required");
  const request = CancelTaskRequestSchema.parse({
    project_id: baseline.project.id,
    expected_revision: baseline.project.revision,
    payload: {
      task_id: baseline.task.id,
      expected_task_revision: baseline.task.revision,
      cancellation_reason_key: reason,
      ...(/^\p{White_Space}*$/u.test(note) ? {} : { note }),
    },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    taskId: request.payload.task_id,
  });
}
