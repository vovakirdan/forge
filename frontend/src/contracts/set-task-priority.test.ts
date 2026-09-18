import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "./fixtures.ts";
import { SetTaskPriorityRequestSchema } from "./set-task-priority.ts";
import { TaskCommandReceiptSchema } from "./task-command.ts";

const request = {
  project_id: ids.project,
  expected_revision: 3,
  payload: { task_id: ids.task, expected_task_revision: 7, priority: "high" },
};
test("priority command accepts only its narrow wire shape without coercion", () => {
  assert.deepEqual(SetTaskPriorityRequestSchema.parse(request), request);
  for (const value of [
    { ...request, actor: ids.actor },
    { ...request, expected_revision: "3" },
    { ...request, expected_revision: Number.MAX_SAFE_INTEGER },
    { ...request, project_id: "wrong" },
    ...[
      { priority: "" },
      { priority: "High" },
      { priority: "x".repeat(65) },
      { expected_task_revision: 0 },
      { expected_task_revision: Number.MAX_SAFE_INTEGER },
      { task_id: ids.task.replace("7000", "4000") },
      { patch: {} },
    ].map((payload) => ({ ...request, payload: { ...request.payload, ...payload } })),
  ])
    assert.equal(SetTaskPriorityRequestSchema.safeParse(value).success, false);
});
test("task command receipts require actual events and a Task resource", () => {
  const receipt = {
    command_id: ids.actor,
    status: "applied",
    project_revision: 4,
    event_ids: [ids.artifact],
    resource: { kind: "task", id: ids.task },
  };
  assert.equal(TaskCommandReceiptSchema.safeParse(receipt).success, true);
  for (const change of [
    { event_ids: [] },
    { status: "queued" },
    { resource: { kind: "project", id: ids.project } },
    { extra: true },
  ])
    assert.equal(TaskCommandReceiptSchema.safeParse({ ...receipt, ...change }).success, false);
});
