import assert from "node:assert/strict";
import { test } from "node:test";
import { PipelineVersionViewSchema, TaskSummaryViewSchema, resolveTaskStage } from "./index.ts";
import {
  analysisPipelineFixture,
  deliveryPipelineFixture,
  ids,
  taskSummaryFixture,
} from "./fixtures.ts";

test("resolves the actual waiting stage, not entry or array order", () => {
  const task = TaskSummaryViewSchema.parse(taskSummaryFixture);
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  const result = resolveTaskStage(task, pipeline);
  assert.equal(result.status, "resolved");
  if (result.status === "resolved") {
    assert.equal(result.stage.id, "choose_path");
    assert.notEqual(result.stage.id, pipeline.entry_stage_id);
  }
});

test("pinned v1 resolves even after catalog default/latest become v2 and Pipeline is deleted", () => {
  const task = { current_stage_id: "work", pipeline_version_id: ids.version1 };
  const pipeline = PipelineVersionViewSchema.parse(deliveryPipelineFixture);
  const result = resolveTaskStage(task, pipeline);
  assert.equal(result.status, "resolved");
  if (result.status === "resolved") assert.equal(result.stage.id, "work");
  assert.deepEqual(resolveTaskStage(task, { ...pipeline, id: ids.version2, version: 2 }), {
    status: "unavailable",
    reason: "version_mismatch",
  });
});

test("null current stage is no_stage even without a loaded or matching version", () => {
  const task = { current_stage_id: null, pipeline_version_id: ids.version1 };
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  for (const loaded of [undefined, null, pipeline]) {
    assert.deepEqual(resolveTaskStage(task, loaded), { status: "no_stage" });
  }
});

test("missing or mismatched versions and missing stages have distinct unavailable reasons", () => {
  const task = TaskSummaryViewSchema.parse(taskSummaryFixture);
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  for (const loaded of [null, undefined]) {
    assert.deepEqual(resolveTaskStage(task, loaded), {
      status: "unavailable",
      reason: "version_not_loaded",
    });
  }
  assert.deepEqual(resolveTaskStage(task, { ...pipeline, id: ids.version1 }), {
    status: "unavailable",
    reason: "version_mismatch",
  });
  assert.deepEqual(resolveTaskStage({ ...task, current_stage_id: "unavailable_stage" }, pipeline), {
    status: "unavailable",
    reason: "stage_not_found",
  });
});

test("resolution does not mutate inputs, apply transitions or manufacture Run activity", () => {
  const task = TaskSummaryViewSchema.parse(taskSummaryFixture);
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  const before = structuredClone({ task, pipeline });
  Object.freeze(task);
  pipeline.stages.forEach(Object.freeze);
  pipeline.transitions.forEach(Object.freeze);
  Object.freeze(pipeline.stages);
  Object.freeze(pipeline.transitions);
  Object.freeze(pipeline);
  const result = resolveTaskStage(task, pipeline);
  assert.equal(result.status, "resolved");
  assert.deepEqual({ task, pipeline }, before);
  assert.deepEqual(Object.keys(result).sort(), ["stage", "status"]);
});
