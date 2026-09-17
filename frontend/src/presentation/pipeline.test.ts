import assert from "node:assert/strict";
import { test } from "node:test";
import { PipelineVersionViewSchema } from "../contracts/pipeline.ts";
import { deliveryPipelineFixture, ids } from "../contracts/fixtures.ts";
import { presentPipelineVersion, pipelineStageLabel, pipelineTargetLabel } from "./pipeline.ts";
import { freezeFixture } from "./test-helpers.ts";

test("Pipeline catalog flags are independent of version definition and soft deletion", () => {
  const pipeline = freezeFixture(
    PipelineVersionViewSchema.parse({
      ...deliveryPipelineFixture,
      default_version_id: ids.version1,
    }),
  );
  const result = presentPipelineVersion(pipeline);
  assert.strictEqual(result.source, pipeline);
  assert.deepEqual(result.presentation, {
    isDefault: true,
    isLatest: false,
    deletionLabel: "Soft deleted",
  });
  assert.deepEqual(presentPipelineVersion(pipeline), result);
  const latest = PipelineVersionViewSchema.parse({
    ...pipeline,
    id: ids.version2,
    version: 2,
    deleted_at: null,
  });
  assert.deepEqual(presentPipelineVersion(latest).presentation, {
    isDefault: false,
    isLatest: true,
    deletionLabel: "Not deleted",
  });
});

test("Stage references resolve by exact ID without traversal or name fallback", () => {
  const pipeline = PipelineVersionViewSchema.parse(deliveryPipelineFixture);
  assert.equal(pipelineStageLabel(pipeline, "work"), "Work (work)");
  assert.equal(pipelineStageLabel(pipeline, "Work"), "Work — stage unavailable");
  assert.equal(pipelineStageLabel(pipeline, "unknown"), "unknown — stage unavailable");
  assert.equal(
    pipelineTargetLabel(pipeline, { kind: "stage", stage_id: "review" }),
    "Review (review)",
  );
  assert.equal(
    pipelineTargetLabel(pipeline, { kind: "stage", stage_id: "gone" }),
    "gone — stage unavailable",
  );
  assert.equal(pipelineTargetLabel(pipeline, { kind: "done" }), "done (terminal)");
  assert.equal(pipelineTargetLabel(pipeline, { kind: "cancelled" }), "cancelled (terminal)");
});
