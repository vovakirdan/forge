import assert from "node:assert/strict";
import { test } from "node:test";
import {
  PipelineVersionViewSchema,
  TaskDetailViewSchema,
  TaskSummaryViewSchema,
  type LifecycleStatus,
  type TaskKind,
} from "../contracts/index.ts";
import {
  analysisPipelineFixture,
  deliveryPipelineFixture,
  ids,
  taskDetailFixture,
  taskSummaryFixture,
} from "../contracts/fixtures.ts";
import { presentTaskDetail, presentTaskSummary } from "./index.ts";
import { freezeFixture } from "./test-helpers.ts";

test("Task labels cover both kinds and every lifecycle independently", () => {
  const kinds: [TaskKind, string][] = [
    ["delivery", "Delivery"],
    ["analysis", "Analysis"],
  ];
  const lifecycles: [LifecycleStatus, string][] = [
    ["draft", "Draft"],
    ["ready", "Ready"],
    ["in_progress", "In progress"],
    ["waiting", "Waiting"],
    ["done", "Done"],
    ["cancelled", "Cancelled"],
  ];
  for (const [kind, kindLabel] of kinds) {
    for (const [lifecycle, lifecycleLabel] of lifecycles) {
      const source = TaskSummaryViewSchema.parse({ ...taskSummaryFixture, kind, lifecycle });
      const result = presentTaskSummary(source);
      assert.strictEqual(result.source, source);
      assert.equal(result.presentation.kindLabel, kindLabel);
      assert.equal(result.presentation.lifecycleLabel, lifecycleLabel);
    }
  }
});

test("Task stage resolves only its pin even after a new default and soft deletion", () => {
  const task = TaskSummaryViewSchema.parse({
    ...taskSummaryFixture,
    kind: "delivery",
    current_stage_id: "review",
    pipeline_version_id: ids.version1,
  });
  const pipeline = PipelineVersionViewSchema.parse(deliveryPipelineFixture);
  assert.notEqual(pipeline.default_version_id, task.pipeline_version_id);
  assert.notEqual(pipeline.deleted_at, null);
  const stage = presentTaskSummary(task, pipeline).presentation.stage;
  assert.equal(stage.status, "resolved");
  if (stage.status === "resolved") {
    assert.strictEqual(stage.stage, pipeline.stages[1]);
    assert.equal(stage.stage.name, "Review");
  }
});

test("Task stage preserves all unavailable reasons without a lifecycle fallback", () => {
  const task = TaskSummaryViewSchema.parse(taskSummaryFixture);
  for (const pipeline of [undefined, null]) {
    assert.deepEqual(presentTaskSummary(task, pipeline).presentation.stage, {
      status: "unavailable",
      reason: "version_not_loaded",
    });
  }
  assert.deepEqual(
    presentTaskSummary(task, PipelineVersionViewSchema.parse(deliveryPipelineFixture)).presentation
      .stage,
    { status: "unavailable", reason: "version_mismatch" },
  );
  const missingStage = TaskSummaryViewSchema.parse({
    ...taskSummaryFixture,
    current_stage_id: "owner_custom_stage",
  });
  assert.deepEqual(
    presentTaskSummary(missingStage, PipelineVersionViewSchema.parse(analysisPipelineFixture))
      .presentation.stage,
    { status: "unavailable", reason: "stage_not_found" },
  );
});

test("No stage stays explicit; cancelled Task does not gain a reason or next action", () => {
  const task = TaskDetailViewSchema.parse({
    ...taskDetailFixture,
    lifecycle: "cancelled",
    current_stage_id: null,
    definition_of_done: null,
    wait_conditions: [],
    artifacts: [],
  });
  for (const pipeline of [
    undefined,
    null,
    PipelineVersionViewSchema.parse(deliveryPipelineFixture),
  ]) {
    assert.deepEqual(presentTaskDetail(task, pipeline).presentation, {
      kindLabel: "Analysis",
      lifecycleLabel: "Cancelled",
      stage: { status: "no_stage" },
      loadedWaitConditionCount: 0,
      loadedArtifactCount: 0,
    });
  }
});

test("Analysis without Git retains arbitrary priority IDs with no invented rank", () => {
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  for (const priority of ["p99", "critical_to_owner", "single_priority"]) {
    const task = TaskSummaryViewSchema.parse({ ...taskSummaryFixture, priority });
    const result = presentTaskSummary(task, pipeline);
    assert.equal(result.source.priority, priority);
    assert.deepEqual(Object.keys(result.presentation).sort(), [
      "kindLabel",
      "lifecycleLabel",
      "stage",
    ]);
    assert.equal(result.presentation.stage.status, "resolved");
    if (result.presentation.stage.status === "resolved") {
      assert.equal(result.presentation.stage.stage.workspace, null);
    }
  }
});

test("A project property resembling a cancellation reason stays uninterpreted", () => {
  const task = TaskDetailViewSchema.parse({
    ...taskDetailFixture,
    lifecycle: "cancelled",
    current_stage_id: null,
    definition_of_done: null,
    properties: {
      cancellation_reason: { type: "text", value: "Not a canonical cancellation reason" },
    },
    wait_conditions: [],
  });
  const result = presentTaskDetail(task);
  assert.strictEqual(result.source.properties, task.properties);
  assert.deepEqual(result.presentation, {
    kindLabel: "Analysis",
    lifecycleLabel: "Cancelled",
    stage: { status: "no_stage" },
    loadedWaitConditionCount: 0,
    loadedArtifactCount: 1,
  });
});

test("Task detail reuses summary and counts only its loaded waits and artifacts", () => {
  const task = TaskDetailViewSchema.parse(taskDetailFixture);
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  const detail = presentTaskDetail(task, pipeline);
  assert.strictEqual(detail.source, task);
  assert.deepEqual(detail.presentation, {
    ...presentTaskSummary(task, pipeline).presentation,
    loadedWaitConditionCount: 2,
    loadedArtifactCount: 1,
  });
  assert.strictEqual(detail.source.wait_conditions, task.wait_conditions);
  assert.strictEqual(detail.source.artifacts, task.artifacts);
});

test("Task presentation preserves opaque JSON and is repeatable on frozen source and pipeline", () => {
  const raw = JSON.parse(JSON.stringify(taskDetailFixture));
  raw.extra = JSON.parse('{"__proto__":{"retained":true},"constructor":{"note":"opaque"}}');
  raw.artifacts[0].body = {
    storage: "object_reference",
    object_key: "not-a-fetch-request",
    nested: raw.extra,
  };
  raw.artifacts[0].metadata = raw.extra;
  const task = freezeFixture(TaskDetailViewSchema.parse(raw));
  const pipeline = freezeFixture(PipelineVersionViewSchema.parse(analysisPipelineFixture));
  const before = JSON.stringify({ task, pipeline });
  const result = presentTaskDetail(task, pipeline);
  assert.strictEqual(result.source, task);
  assert.strictEqual(result.source["extra"], raw.extra);
  const artifact = result.source.artifacts[0];
  assert.ok(artifact);
  assert.strictEqual(artifact.body, raw.artifacts[0].body);
  assert.deepEqual(result, presentTaskDetail(task, pipeline));
  assert.equal(JSON.stringify({ task, pipeline }), before);
});
