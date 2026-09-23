import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "./fixtures.ts";
import { CreateTaskRequestSchema, DraftDefinitionOfDoneSchema } from "./create-task.ts";

const request = {
  project_id: ids.project,
  expected_revision: 8,
  payload: {
    title: "A draft",
    description: "",
    definition_of_done: null,
    kind: "analysis",
    priority: "normal",
    pipeline_version_id: ids.version1,
    properties: {},
  },
};
test("creation accepts a pinned draft with optional DoD and typed properties", () => {
  assert.deepEqual(CreateTaskRequestSchema.parse(request), request);
  assert.equal(
    CreateTaskRequestSchema.safeParse({
      ...request,
      payload: { ...request.payload, properties: { impact: { type: "enum", value: "high" } } },
    }).success,
    true,
  );
  assert.equal(
    CreateTaskRequestSchema.safeParse({
      ...request,
      payload: { ...request.payload, kind: "delivery", definition_of_done: "Evidence attached" },
    }).success,
    true,
  );
  for (const payload of [
    { kind: "" },
    { title: "   " },
    { pipeline_version_id: "" },
    { pipeline_id: ids.pipeline },
    { id: ids.task },
    { lifecycle: "ready" },
    { properties: { required: true } },
    { definition_of_done: "  " },
    { priority: "" },
  ])
    assert.equal(
      CreateTaskRequestSchema.safeParse({ ...request, payload: { ...request.payload, ...payload } })
        .success,
      false,
    );
  assert.equal(
    CreateTaskRequestSchema.safeParse({ ...request, expected_revision: Number.MAX_SAFE_INTEGER })
      .success,
    false,
  );
  assert.equal(CreateTaskRequestSchema.safeParse({ ...request, actor: ids.actor }).success, false);
});
test("optional DoD follows Core scalar and nonblank limits without trimming", () => {
  assert.equal(DraftDefinitionOfDoneSchema.parse("  Preserve spaces  "), "  Preserve spaces  ");
  assert.equal(DraftDefinitionOfDoneSchema.safeParse("😀".repeat(20_000)).success, true);
  for (const invalid of ["", "\u2003", "x".repeat(20_001), "\ud800"])
    assert.equal(DraftDefinitionOfDoneSchema.safeParse(invalid).success, false);
});
