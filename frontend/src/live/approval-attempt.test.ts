import assert from "node:assert/strict";
import { test } from "node:test";
import {
  projectFixture,
  taskDetailFixture,
  deliveryPipelineFixture,
} from "../contracts/fixtures.ts";
import { ProjectViewSchema } from "../contracts/project.ts";
import { TaskDetailViewSchema } from "../contracts/task.ts";
import { PipelineVersionViewSchema } from "../contracts/pipeline.ts";
import { createApprovalAttempt } from "./approval-attempt.ts";
import { createDraftAttempt, draftChanged, rebaseDraftFields } from "./draft-attempt.ts";

const baseline = {
  project: ProjectViewSchema.parse(projectFixture),
  task: TaskDetailViewSchema.parse({
    ...taskDetailFixture,
    lifecycle: "draft",
    definition_of_done: "saved DoD",
    pipeline_version_id: deliveryPipelineFixture.id,
  }),
  pipeline: PipelineVersionViewSchema.parse(deliveryPipelineFixture),
};
const key = "10000000-0000-4000-8000-000000000001";
test("approval freezes saved revisions, not lifecycle assumptions or unsaved data", () => {
  const attempt = createApprovalAttempt(baseline, key);
  assert.equal(Object.isFrozen(attempt), true);
  assert.deepEqual(JSON.parse(attempt.body), {
    project_id: baseline.project.id,
    expected_revision: baseline.project.revision,
    payload: { task_id: baseline.task.id, expected_task_revision: baseline.task.revision },
  });
  assert.equal(attempt.key, key);
  assert.doesNotThrow(() =>
    createApprovalAttempt(
      { ...baseline, pipeline: { ...baseline.pipeline, deleted_at: "2026-09-18T00:00:00Z" } },
      key,
    ),
  );
  assert.throws(() =>
    createApprovalAttempt(
      { ...baseline, task: { ...baseline.task, definition_of_done: null } },
      key,
    ),
  );
  assert.throws(() =>
    createApprovalAttempt({ ...baseline, task: { ...baseline.task, lifecycle: "ready" } }, key),
  );
  assert.throws(() =>
    createApprovalAttempt(
      { ...baseline, pipeline: { ...baseline.pipeline, id: baseline.project.id } },
      key,
    ),
  );
});
test("DoD changes preserve omission, clear blank values and retain edited intent on conflict", () => {
  assert.deepEqual(
    JSON.parse(
      createDraftAttempt(baseline, { ...baseline.task, definition_of_done: " \n" }, key).body,
    ).payload.patch,
    { definition_of_done: null },
  );
  assert.deepEqual(
    JSON.parse(
      createDraftAttempt(
        baseline,
        { title: "new title", description: baseline.task.description },
        key,
      ).body,
    ).payload.patch,
    { title: "new title" },
  );
  const empty = { ...baseline, task: { ...baseline.task, definition_of_done: null } };
  assert.equal(draftChanged({ ...empty.task, definition_of_done: " " }, empty.task), false);
  const after = { ...baseline.task, title: "server title", definition_of_done: "server DoD" };
  assert.equal(
    rebaseDraftFields({ ...baseline.task, title: "mine" }, baseline.task, after).definition_of_done,
    "server DoD",
  );
  const retained = rebaseDraftFields(
    { ...baseline.task, definition_of_done: "" },
    baseline.task,
    after,
  );
  assert.equal(retained.definition_of_done, "");
  assert.equal(retained.title, "server title");
});
