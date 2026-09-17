import type { PipelineStageView, PipelineVersionView } from "./pipeline.ts";
import type { TaskSummaryView } from "./task.ts";

export type TaskStageResolution =
  | { status: "resolved"; stage: PipelineStageView }
  | { status: "no_stage" }
  | {
      status: "unavailable";
      reason: "version_not_loaded" | "version_mismatch" | "stage_not_found";
    };

/** Resolve only the explicitly pinned stage; never guess from lifecycle or defaults. */
export function resolveTaskStage(
  task: Pick<TaskSummaryView, "pipeline_version_id" | "current_stage_id">,
  pipeline: PipelineVersionView | null | undefined,
): TaskStageResolution {
  if (task.current_stage_id === null) return { status: "no_stage" };
  if (pipeline == null) return { status: "unavailable", reason: "version_not_loaded" };
  if (pipeline.id !== task.pipeline_version_id) {
    return { status: "unavailable", reason: "version_mismatch" };
  }
  const stage = pipeline.stages.find((item) => item.id === task.current_stage_id);
  return stage
    ? { status: "resolved", stage }
    : { status: "unavailable", reason: "stage_not_found" };
}
