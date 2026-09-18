import assert from "node:assert/strict";
import { test } from "node:test";
import { ApproveTaskRequestSchema } from "./approve-task.ts";
import { DraftPatchSchema, DraftDefinitionOfDoneSchema } from "./amend-draft.ts";
import { ids } from "./fixtures.ts";

test("approval carries only explicit revisions and Task identity", () => {
  const request = {
    project_id: ids.project,
    expected_revision: 3,
    payload: { task_id: ids.task, expected_task_revision: 7 },
  };
  assert.deepEqual(ApproveTaskRequestSchema.parse(request), request);
  for (const invalid of [
    { ...request, actor: ids.actor },
    { ...request, expected_revision: Number.MAX_SAFE_INTEGER },
    { ...request, payload: { ...request.payload, definition_of_done: "not saved" } },
    { ...request, payload: { ...request.payload, lifecycle: "ready" } },
    { ...request, payload: { task_id: ids.task } },
  ])
    assert.equal(ApproveTaskRequestSchema.safeParse(invalid).success, false);
});
test("DoD patch distinguishes omission, explicit clear and replacement", () => {
  assert.deepEqual(DraftPatchSchema.parse({ title: "new" }), { title: "new" });
  assert.deepEqual(DraftPatchSchema.parse({ definition_of_done: null }), {
    definition_of_done: null,
  });
  assert.deepEqual(DraftPatchSchema.parse({ definition_of_done: " saved " }), {
    definition_of_done: " saved ",
  });
  assert.equal(DraftPatchSchema.safeParse({}).success, false);
  for (const invalid of ["", " \n", "\u0085", "\uD800", "😀".repeat(20_001)])
    assert.equal(DraftDefinitionOfDoneSchema.safeParse(invalid).success, false);
  assert.equal(DraftDefinitionOfDoneSchema.parse("😀".repeat(20_000)), "😀".repeat(20_000));
  assert.equal(DraftDefinitionOfDoneSchema.parse("\uFEFF"), "\uFEFF");
});
