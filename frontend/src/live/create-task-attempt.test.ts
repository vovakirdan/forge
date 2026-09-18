import assert from "node:assert/strict";
import { test } from "node:test";
import { analysisPipelineFixture, ids } from "../contracts/fixtures.ts";
import { PipelineVersionViewSchema } from "../contracts/pipeline.ts";
import { PrioritySchemeViewSchema } from "../contracts/priority-scheme.ts";
import {
  createTaskAttempt,
  initialCreateTaskFields,
  pipelineAllowsCreation,
} from "./create-task-attempt.ts";

const scheme = PrioritySchemeViewSchema.parse({
  project_id: ids.project,
  project_revision: 12,
  default_level_id: "normal",
  levels: [
    { id: "normal", display_name: "Normal", rank: 1, retired: false },
    { id: "old", display_name: "Old", rank: 2, retired: true },
  ],
});
const pipeline = PipelineVersionViewSchema.parse({ ...analysisPipelineFixture, deleted_at: null });
const key = "10000000-0000-4000-8000-000000000001";
const fields = {
  ...initialCreateTaskFields(scheme),
  title: "  Human draft  ",
  kind: "analysis" as const,
  pipelineVersionId: pipeline.id,
};
test("creation requires explicit kind and version but seeds only the validated priority default", () => {
  const initial = initialCreateTaskFields(scheme);
  assert.equal(initial.priority, "normal");
  assert.equal(initial.kind, "");
  assert.equal(initial.pipelineVersionId, "");
  assert.throws(() => createTaskAttempt(scheme, pipeline, initial, key));
});
test("creation freezes exact intended fields with current scheme revision and no client Task identity", () => {
  const attempt = createTaskAttempt(scheme, pipeline, fields, key);
  assert.equal(Object.isFrozen(attempt), true);
  assert.equal(attempt.key, key);
  assert.deepEqual(JSON.parse(attempt.body), {
    project_id: ids.project,
    expected_revision: 12,
    payload: {
      title: "  Human draft  ",
      description: "",
      definition_of_done: null,
      kind: "analysis",
      priority: "normal",
      pipeline_version_id: pipeline.id,
      properties: {},
    },
  });
  const afterRefresh = createTaskAttempt(
    { ...scheme, project_revision: 13 },
    pipeline,
    fields,
    key,
  );
  assert.equal(JSON.parse(afterRefresh.body).expected_revision, 13);
  assert.equal(JSON.parse(attempt.body).expected_revision, 12);
});
test("deleted, incompatible, mismatched or unavailable Pipeline and priority choices fail closed", () => {
  assert.equal(pipelineAllowsCreation(undefined, fields), false);
  for (const selected of [
    { ...pipeline, deleted_at: "2026-09-18T10:00:00Z" },
    { ...pipeline, task_kinds: [] },
    { ...pipeline, id: ids.version1 },
  ])
    assert.throws(() => createTaskAttempt(scheme, selected, fields, key));
  for (const priority of ["old", "missing"])
    assert.throws(() => createTaskAttempt(scheme, pipeline, { ...fields, priority }, key));
  assert.throws(() =>
    createTaskAttempt(scheme, pipeline, { ...fields, definitionOfDone: " " }, key),
  );
});
