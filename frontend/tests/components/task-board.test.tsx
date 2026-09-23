import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { TaskSummaryViewSchema } from "../../src/contracts/task.ts";
import { ids, taskSummaryFixture } from "../../src/contracts/fixtures.ts";
import { TaskBoard } from "../../src/live/TaskBoard.tsx";

test("Board renders exact pinned stage lanes and separate lifecycle on accessible cards", () => {
  const tasks = [
    TaskSummaryViewSchema.parse({
      ...taskSummaryFixture,
      id: ids.task,
      key: "TASK-001",
      title: "<unsafe>",
      current_stage_id: "custom_review",
      lifecycle: "waiting",
    }),
    TaskSummaryViewSchema.parse({
      ...taskSummaryFixture,
      id: ids.wait1,
      key: "TASK-002",
      current_stage_id: "custom_review",
      lifecycle: "done",
      pipeline_version_id: ids.version2,
    }),
  ];
  const html = renderToStaticMarkup(
    <TaskBoard tasks={tasks} priorities={{ status: "unavailable" }} onOpenTask={() => {}} />,
  );

  assert.match(html, /aria-label="Task board"/);
  assert.equal((html.match(/Stage ID: custom_review/g) ?? []).length, 2);
  assert.match(html, /Lifecycle<\/dt><dd>Waiting/);
  assert.match(html, /Lifecycle<\/dt><dd>Done/);
  assert.match(html, /aria-label="Open TASK-001"/);
  assert.match(html, /aria-controls="task-detail"/);
  assert.match(html, /&lt;unsafe&gt;/);
  assert.doesNotMatch(html, /<unsafe>/);
  assert.doesNotMatch(html, /Run activity: none|No runs/i);
});
