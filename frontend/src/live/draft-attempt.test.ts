import assert from "node:assert/strict";
import { test } from "node:test";
import { ProjectViewSchema } from "../contracts/project.ts";
import { TaskDetailViewSchema } from "../contracts/task.ts";
import { projectFixture, taskDetailFixture } from "../contracts/fixtures.ts";
import { createDraftAttempt, draftChanged, rebaseDraftFields } from "./draft-attempt.ts";
import { createLeaveGuard, draftLeaveWarning } from "./leave-guard.ts";

const baseline = {
  project: ProjectViewSchema.parse(projectFixture),
  task: TaskDetailViewSchema.parse({ ...taskDetailFixture, lifecycle: "draft" }),
};
const key = "10000000-0000-4000-8000-000000000001";

test("one attempt freezes exact bytes, revisions and key with changed fields only", () => {
  const fields = { title: "  New title  ", description: baseline.task.description };
  const attempt = createDraftAttempt(baseline, fields, key);
  const body: unknown = JSON.parse(attempt.body);
  assert.deepEqual(body, {
    project_id: baseline.project.id,
    expected_revision: baseline.project.revision,
    payload: {
      task_id: baseline.task.id,
      expected_task_revision: baseline.task.revision,
      patch: { title: "  New title  " },
    },
  });
  fields.title = "later edit";
  assert.equal(attempt.body.includes("later edit"), false);
  assert.equal(Object.isFrozen(attempt), true);
  assert.equal(attempt.key, key);
  assert.equal(draftChanged(baseline.task, baseline.task), false);
  const emptyDescription = createDraftAttempt(
    baseline,
    { title: baseline.task.title, description: "" },
    key,
  );
  assert.deepEqual(JSON.parse(emptyDescription.body).payload.patch, { description: "" });
});

test("cannot create an empty save, non-draft save or invalid retry key", () => {
  assert.throws(() => createDraftAttempt(baseline, baseline.task, key));
  assert.throws(() =>
    createDraftAttempt(
      { ...baseline, task: { ...baseline.task, lifecycle: "done" } },
      { title: "new", description: "" },
      key,
    ),
  );
  assert.throws(() => createDraftAttempt(baseline, { title: "new", description: "" }, "bad"));
});

test("refresh retains edited-field intent, picks up untouched fields and never auto-saves", () => {
  const before = { title: "old title", description: "old description" };
  const after = { title: "new server title", description: "new server description" };
  assert.deepEqual(rebaseDraftFields({ ...before, title: "my title" }, before, after), {
    title: "my title",
    description: after.description,
  });
  assert.deepEqual(rebaseDraftFields({ ...before, description: "" }, before, after), {
    title: after.title,
    description: "",
  });
  assert.deepEqual(rebaseDraftFields(before, before, after), after);
});

test("leave guard requires explicit confirmation and removes only the disposed registration", () => {
  const notices: string[] = [];
  let allowed = false;
  const guard = createLeaveGuard((message) => {
    notices.push(message);
    return allowed;
  });
  assert.equal(guard.canLeave(), true);
  assert.equal(guard.hasPending(), false);
  const removeClean = guard.register(() => false);
  const removeDirty = guard.register(() => true);
  assert.equal(guard.hasPending(), true);
  assert.equal(guard.canLeave(), false);
  allowed = true;
  assert.equal(guard.canLeave(), true);
  assert.deepEqual(notices, [draftLeaveWarning, draftLeaveWarning]);
  removeClean();
  removeDirty();
  assert.equal(guard.hasPending(), false);
  assert.equal(guard.canLeave(), true);
  assert.equal(notices.length, 2);
});
