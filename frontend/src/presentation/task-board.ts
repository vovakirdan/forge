import type { TaskSummaryView } from "../contracts/task.ts";

export type TaskBoardLane = {
  key: string;
  stageId: string | null;
  pipelineVersionId: string | null;
  tasks: TaskSummaryView[];
};

/** Group only the current server page; equal stage IDs in different pins are distinct. */
export function taskBoardLanes(tasks: readonly TaskSummaryView[]): TaskBoardLane[] {
  const lanes = new Map<string, TaskBoardLane>();
  for (const task of tasks) {
    const stageId = task.current_stage_id;
    const pipelineVersionId = stageId === null ? null : task.pipeline_version_id;
    const key = JSON.stringify([pipelineVersionId, stageId]);
    let lane = lanes.get(key);
    if (!lane) {
      lane = { key, stageId, pipelineVersionId, tasks: [] };
      lanes.set(key, lane);
    }
    lane.tasks.push(task);
  }
  return [...lanes.values()];
}
