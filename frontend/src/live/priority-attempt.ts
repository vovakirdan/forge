import type { LifecycleStatus } from "../contracts/common.ts";
import type { PrioritySchemeView } from "../contracts/priority-scheme.ts";
import type { TaskDetailView } from "../contracts/task.ts";
import { SetTaskPriorityRequestSchema } from "../contracts/set-task-priority.ts";
import { IdempotencyKeySchema, type TaskCommandAttempt } from "../contracts/task-command.ts";

export type PriorityBaseline = { scheme: PrioritySchemeView; task: TaskDetailView };
export function canChangePriority(lifecycle: LifecycleStatus) {
  return lifecycle !== "done" && lifecycle !== "cancelled";
}
export function activePriority(scheme: PrioritySchemeView, priority: string) {
  return scheme.levels.some((level) => level.id === priority && !level.retired);
}
export function createPriorityAttempt(
  baseline: PriorityBaseline,
  priority: string,
  key: string = crypto.randomUUID(),
): TaskCommandAttempt {
  if (
    !canChangePriority(baseline.task.lifecycle) ||
    priority === baseline.task.priority ||
    !activePriority(baseline.scheme, priority)
  )
    throw new Error("Priority change is unavailable");
  const request = SetTaskPriorityRequestSchema.parse({
    project_id: baseline.scheme.project_id,
    expected_revision: baseline.scheme.project_revision,
    payload: {
      task_id: baseline.task.id,
      expected_task_revision: baseline.task.revision,
      priority,
    },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    taskId: baseline.task.id,
  });
}
