import assert from "node:assert/strict";
import { test } from "node:test";
import { ids, taskDetailFixture } from "./fixtures.ts";
import { CancellationNoteSchema, CancelTaskRequestSchema } from "./cancel-task.ts";
import { CancellationReasonsViewSchema } from "./cancellation-reasons.ts";
import { TaskDetailViewSchema } from "./task.ts";

test("cancellation input enforces explicit reason, strict fields and Unicode note bounds", () => {
  const value = {
    project_id: ids.project,
    expected_revision: 1,
    payload: {
      task_id: ids.task,
      expected_task_revision: 1,
      cancellation_reason_key: "unspecified",
    },
  };
  assert.equal(CancelTaskRequestSchema.safeParse(value).success, true);
  for (const payload of [
    { ...value.payload, cancellation_reason_key: "" },
    { ...value.payload, note: " " },
    { ...value.payload, actor: ids.actor },
  ])
    assert.equal(CancelTaskRequestSchema.safeParse({ ...value, payload }).success, false);
  assert.equal(CancellationNoteSchema.safeParse("😀".repeat(20_000)).success, true);
  assert.equal(CancellationNoteSchema.safeParse("x".repeat(20_001)).success, false);
  assert.equal(CancellationNoteSchema.safeParse("\ud800").success, false);
});
test("cancellation catalog rejects duplicate reason IDs and task metadata is explicitly nullable", () => {
  const reason = { id: "unspecified", display_name: "Unspecified", retired: false };
  assert.equal(
    CancellationReasonsViewSchema.safeParse({
      project_id: ids.project,
      project_revision: 1,
      reasons: [reason, reason],
    }).success,
    false,
  );
  assert.equal(TaskDetailViewSchema.safeParse(taskDetailFixture).success, true);
  assert.equal(
    TaskDetailViewSchema.safeParse({ ...taskDetailFixture, cancellation: undefined }).success,
    false,
  );
  assert.equal(
    TaskDetailViewSchema.safeParse({
      ...taskDetailFixture,
      cancellation: {
        reason_id: "unspecified",
        note: null,
        cancelled_by: { kind: "human", id: ids.actor },
        cancelled_at: "2026-09-18T00:00:00Z",
      },
    }).success,
    true,
  );
});
