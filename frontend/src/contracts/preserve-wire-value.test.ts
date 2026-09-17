import assert from "node:assert/strict";
import { test } from "node:test";
import { z } from "zod";
import {
  ActorReferenceSchema,
  ArtifactRequirementViewSchema,
  ArtifactViewSchema,
  ForgeReferenceSchema,
  PipelineStageViewSchema,
  PipelineTargetViewSchema,
  PipelineTransitionViewSchema,
  PipelineVersionListResponseSchema,
  PipelineVersionViewSchema,
  ProjectViewSchema,
  StageAcceptancePolicySchema,
  StageWorkspaceRequirementsSchema,
  SystemStageActionSchema,
  TaskDetailViewSchema,
  TaskListResponseSchema,
  TaskPropertyValueSchema,
  TaskSummaryViewSchema,
  WaitConditionViewSchema,
} from "./index.ts";
import {
  deliveryPipelineFixture,
  ids,
  projectFixture,
  taskDetailFixture,
  taskSummaryFixture,
} from "./fixtures.ts";

test("additive JSON keys survive every public object schema, including nested objects", () => {
  const extra: Record<string, unknown> = JSON.parse(
    '{"__proto__":{"literal":true},"constructor":{"__proto__":null}}',
  );
  const cases: [z.ZodTypeAny, object | undefined][] = [
    [ProjectViewSchema, projectFixture],
    [TaskSummaryViewSchema, taskSummaryFixture],
    [TaskDetailViewSchema, taskDetailFixture],
    [TaskListResponseSchema, { items: [taskSummaryFixture] }],
    [ArtifactViewSchema, taskDetailFixture.artifacts[0]],
    [WaitConditionViewSchema, taskDetailFixture.wait_conditions[0]],
    [ActorReferenceSchema, { kind: "human", id: ids.actor }],
    [TaskPropertyValueSchema, { type: "text", value: "x" }],
    [ForgeReferenceSchema, { reference_type: "task", reference_id: ids.task }],
    [PipelineVersionViewSchema, deliveryPipelineFixture],
    [PipelineVersionListResponseSchema, { items: [deliveryPipelineFixture] }],
    [PipelineStageViewSchema, deliveryPipelineFixture.stages[0]],
    [PipelineTransitionViewSchema, deliveryPipelineFixture.transitions[0]],
    [PipelineTargetViewSchema, { kind: "done" }],
    [ArtifactRequirementViewSchema, { kind: "report", minimum_count: 1, scope: "current_stage" }],
    [StageWorkspaceRequirementsSchema, { kind: "any", access: "read_only" }],
    [
      StageAcceptancePolicySchema,
      { kind: "candidate_review", independent: true, verdicts: { ok: "accepted" } },
    ],
    [
      SystemStageActionSchema,
      {
        kind: "git_integration",
        outcomes: { applied: "ok", no_changes: "none", stale_base: "work" },
        required_review_stages: [],
      },
    ],
  ];
  for (const [schema, fixture] of cases) {
    assert.ok(fixture);
    const raw: unknown = JSON.parse(JSON.stringify({ ...fixture, ...extra }));
    assert.deepEqual(schema.parse(raw), raw);
  }
  const nested = {
    ...taskDetailFixture,
    artifacts: [{ ...taskDetailFixture.artifacts[0], ...extra }],
    properties: { custom: { type: "text", value: "x", ...extra } },
    wait_conditions: [
      {
        ...taskDetailFixture.wait_conditions[0],
        ...extra,
        created_by: { kind: "core", id: ids.actor, ...extra },
      },
    ],
  };
  assert.deepEqual(TaskDetailViewSchema.parse(nested), nested);
});

test("preservation retains validation errors and their nested paths", () => {
  const result = TaskListResponseSchema.safeParse({
    items: [{ ...taskSummaryFixture, revision: "seven" }],
  });
  assert.equal(result.success, false);
  if (!result.success) {
    assert.deepEqual(result.error.issues[0]?.path, ["items", 0, "revision"]);
  }
});
