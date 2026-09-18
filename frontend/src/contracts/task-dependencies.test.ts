import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "./fixtures.ts";
import { TaskDependencyListResponseSchema } from "./task-dependencies.ts";

const item = {
  blocker_task_id: ids.actor,
  blocked_task_id: ids.task,
  required_condition: "task_done",
  related_task: { id: ids.actor, key: "TASK-002", title: "Related task", lifecycle: "done" },
  condition_state: "satisfied",
};
test("dependency pages preserve known and future wire fields without deriving condition state", () => {
  const page = { items: [{ ...item, future: { hint: "kept" } }], next_cursor: "opaque+/=" };
  assert.deepEqual(TaskDependencyListResponseSchema.parse(page), page);
  assert.deepEqual(TaskDependencyListResponseSchema.parse({ items: [] }), { items: [] });
  // Lifecycle of the related (blocked) Task cannot determine the blocker's condition.
  assert.equal(
    TaskDependencyListResponseSchema.safeParse({ items: [{ ...item, condition_state: "pending" }] })
      .success,
    true,
  );
});
test("dependency schemas reject unknown semantics and malformed references", () => {
  for (const invalid of [
    { ...item, required_condition: "stage_reached" },
    { ...item, condition_state: "ready" },
    { ...item, blocker_task_id: "bad" },
    { ...item, related_task: { ...item.related_task, lifecycle: "running" } },
    { ...item, related_task: { ...item.related_task, key: "" } },
  ])
    assert.equal(TaskDependencyListResponseSchema.safeParse({ items: [invalid] }).success, false);
  assert.equal(
    TaskDependencyListResponseSchema.safeParse({ items: [], next_cursor: null }).success,
    false,
  );
});
