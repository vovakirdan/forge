import type { LifecycleStatus, TaskKind } from "../contracts/common.ts";
import type { PipelineVersionView } from "../contracts/pipeline.ts";
import { resolveTaskStage } from "../contracts/resolve-task-stage.ts";
import type { TaskDetailView, TaskSummaryView } from "../contracts/task.ts";

const kindLabels = {
  delivery: "Delivery",
  analysis: "Analysis",
} as const satisfies Record<TaskKind, string>;

const lifecycleLabels = {
  draft: "Draft",
  ready: "Ready",
  in_progress: "In progress",
  waiting: "Waiting",
  done: "Done",
  cancelled: "Cancelled",
} as const satisfies Record<LifecycleStatus, string>;

/** Present an already validated read without replacing its canonical fields. */
export function presentTaskSummary<TTask extends TaskSummaryView>(
  task: TTask,
  pipeline?: PipelineVersionView | null,
) {
  return {
    source: task,
    presentation: {
      kindLabel: kindLabels[task.kind],
      lifecycleLabel: lifecycleLabels[task.lifecycle],
      stage: resolveTaskStage(task, pipeline),
    },
  };
}

export function presentTaskDetail<TTask extends TaskDetailView>(
  task: TTask,
  pipeline?: PipelineVersionView | null,
) {
  const summary = presentTaskSummary(task, pipeline);
  return {
    source: task,
    presentation: {
      ...summary.presentation,
      loadedWaitConditionCount: task.wait_conditions.length,
      loadedArtifactCount: task.artifacts.length,
    },
  };
}

export type PresentedTaskSummary = ReturnType<typeof presentTaskSummary>;
export type PresentedTaskDetail = ReturnType<typeof presentTaskDetail>;
