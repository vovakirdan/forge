import assert from "node:assert/strict";
import { test } from "node:test";
import { TaskSummaryViewSchema } from "../contracts/task.ts";
import { ids, taskSummaryFixture } from "../contracts/fixtures.ts";
import { taskBoardLanes } from "./task-board.ts";

test("board groups arbitrary stages by exact Pipeline pin, independently of lifecycle", () => {
  const tasks = [
    {
      ...taskSummaryFixture,
      id: ids.task,
      lifecycle: "waiting",
      current_stage_id: "review_custom",
    },
    { ...taskSummaryFixture, id: ids.wait1, lifecycle: "done", current_stage_id: "review_custom" },
    {
      ...taskSummaryFixture,
      id: ids.wait2,
      lifecycle: "ready",
      current_stage_id: "review_custom",
      pipeline_version_id: ids.version2,
    },
    { ...taskSummaryFixture, id: ids.artifact, lifecycle: "in_progress", current_stage_id: null },
    { ...taskSummaryFixture, id: ids.hook, lifecycle: "cancelled", current_stage_id: null },
  ].map((task) => TaskSummaryViewSchema.parse(task));

  const lanes = taskBoardLanes(tasks);
  assert.equal(lanes.length, 3);
  assert.deepEqual(
    lanes.map((lane) => lane.tasks.map((task) => task.id)),
    [[ids.task, ids.wait1], [ids.wait2], [ids.artifact, ids.hook]],
  );
  assert.deepEqual(
    lanes.map(({ pipelineVersionId, stageId }) => [pipelineVersionId, stageId]),
    [
      [ids.analysisVersion, "review_custom"],
      [ids.version2, "review_custom"],
      [null, null],
    ],
  );
  assert.equal(tasks[0]?.lifecycle, "waiting");
});
