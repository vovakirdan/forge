import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { TaskSummaryViewSchema } from "../../src/contracts/task.ts";
import { taskSummaryFixture } from "../../src/contracts/fixtures.ts";
import { TaskFacts } from "../../src/live/TaskFacts.tsx";

test("TaskFacts keeps lifecycle, stage, and priority distinct and escapes owner text", () => {
  const task = TaskSummaryViewSchema.parse({
    ...taskSummaryFixture,
    title: '<script>alert("owner")</script>',
    lifecycle: "waiting",
    current_stage_id: null,
    priority: "owner_priority",
  });

  const html = renderToStaticMarkup(
    <TaskFacts task={task} priorities={{ status: "unavailable" }} detail />,
  );

  assert.match(html, /Waiting \(waiting\)/);
  assert.match(html, /No current stage/);
  assert.match(html, /Name unavailable \(catalog unavailable\)/);
  assert.match(html, /owner_priority/);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>/);
});
