import assert from "node:assert/strict";
import { test } from "node:test";
import {
  PipelineStageViewSchema,
  PipelineTransitionViewSchema,
  PipelineVersionListResponseSchema,
  PipelineVersionViewSchema,
  StageAcceptancePolicySchema,
  StageWorkspaceRequirementsSchema,
  SystemStageActionSchema,
} from "./index.ts";
import { analysisPipelineFixture, deliveryPipelineFixture, ids } from "./fixtures.ts";

test("Pipeline preserves graph edges, rework and stage order; review and QA are not universal", () => {
  for (const raw of [deliveryPipelineFixture, analysisPipelineFixture]) {
    assert.deepEqual(PipelineVersionViewSchema.parse(raw), raw);
  }
  const delivery = PipelineVersionViewSchema.parse(deliveryPipelineFixture);
  assert.equal(delivery.stages[0]?.id, "integrate");
  assert.equal(delivery.entry_stage_id, "work");
  const analysis = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  assert.equal(
    analysis.stages.some((stage) => ["review", "qa"].includes(stage.id)),
    false,
  );
  assert.equal(
    analysis.transitions.some(
      (transition) =>
        transition.target.kind === "stage" && transition.target.stage_id === "explore",
    ),
    true,
  );
});

test("version identity, catalog revision, default, latest and soft deletion remain distinct", () => {
  const pipeline = PipelineVersionViewSchema.parse(deliveryPipelineFixture);
  assert.equal(pipeline.id, ids.version1);
  assert.equal(pipeline.version, 1);
  assert.equal(pipeline.catalog_revision, 4);
  assert.equal(pipeline.default_version_id, ids.version2);
  assert.equal(pipeline.latest_version, 2);
  assert.equal(pipeline.deleted_at, deliveryPipelineFixture.deleted_at);
});

test("Pipeline and stage nullable fields are present rather than silently defaulted", () => {
  for (const field of Object.keys(deliveryPipelineFixture)) {
    const raw: Record<string, unknown> = { ...deliveryPipelineFixture };
    delete raw[field];
    assert.equal(PipelineVersionViewSchema.safeParse(raw).success, false, field);
  }
  const stage = analysisPipelineFixture.stages[0];
  assert.ok(stage);
  for (const field of Object.keys(stage)) {
    const raw: Record<string, unknown> = { ...stage };
    delete raw[field];
    assert.equal(PipelineStageViewSchema.safeParse(raw).success, false, field);
  }
  assert.equal(
    PipelineVersionViewSchema.safeParse({ ...analysisPipelineFixture, max_stage_visits: null })
      .success,
    true,
  );
});

test("Pipeline rejects unsafe catalog revision, mistyped versions and unknown closed enums", () => {
  for (const patch of [
    { catalog_revision: Number.MAX_SAFE_INTEGER + 1 },
    { catalog_revision: "4" },
    { version: 0 },
    { latest_version: 4_294_967_296 },
    { max_stage_visits: 2.5 },
    { task_kinds: ["research"] },
  ]) {
    assert.equal(
      PipelineVersionViewSchema.safeParse({ ...deliveryPipelineFixture, ...patch }).success,
      false,
    );
  }
  for (const executor_kind of ["employee", "human", "system", "external"]) {
    assert.equal(
      PipelineStageViewSchema.safeParse({ ...analysisPipelineFixture.stages[0], executor_kind })
        .success,
      true,
    );
  }
  assert.equal(
    PipelineStageViewSchema.safeParse({
      ...analysisPipelineFixture.stages[0],
      executor_kind: "runner",
    }).success,
    false,
  );
});

test("HTTP transition targets are tagged and evidence requirements may be absent but not null", () => {
  const transition = { from_stage_id: "work", outcome: "complete", target: { kind: "done" } };
  assert.deepEqual(PipelineTransitionViewSchema.parse(transition), transition);
  assert.deepEqual(
    PipelineTransitionViewSchema.parse({ ...transition, artifact_requirements: [] })
      .artifact_requirements,
    [],
  );
  for (const target of ["done", { stage: "review" }, { kind: "stage" }, { kind: "waiting" }]) {
    assert.equal(PipelineTransitionViewSchema.safeParse({ ...transition, target }).success, false);
  }
  assert.equal(
    PipelineTransitionViewSchema.safeParse({ ...transition, artifact_requirements: null }).success,
    false,
  );
  for (const scope of ["current_stage", "task_history"]) {
    const raw = {
      ...transition,
      artifact_requirements: [
        { kind: "custom_evidence", minimum_count: 2, scope, future_requirement: true },
      ],
    };
    assert.deepEqual(PipelineTransitionViewSchema.parse(raw), raw);
  }
  for (const invalid of [
    { kind: "report", minimum_count: 0, scope: "current_stage" },
    { kind: "report", minimum_count: 1, scope: "global" },
    { kind: "report", minimum_count: 1 },
  ]) {
    assert.equal(
      PipelineTransitionViewSchema.safeParse({ ...transition, artifact_requirements: [invalid] })
        .success,
      false,
    );
  }
});

test("read policies preserve explicit workspace, independence and action outcome mappings", () => {
  for (const kind of ["any", "filesystem", "git"]) {
    for (const access of ["read_only", "read_write"]) {
      const raw = { kind, access, future_workspace: true };
      assert.deepEqual(StageWorkspaceRequirementsSchema.parse(raw), raw);
    }
  }
  assert.equal(StageWorkspaceRequirementsSchema.safeParse({ access: "read_write" }).success, false);
  const review = {
    kind: "candidate_review",
    independent: false,
    verdicts: { custom_result: "inconclusive" },
  };
  assert.deepEqual(StageAcceptancePolicySchema.parse(review), review);
  assert.equal(
    StageAcceptancePolicySchema.safeParse({ kind: "candidate_review", verdicts: {} }).success,
    false,
  );
  assert.equal(
    StageAcceptancePolicySchema.safeParse({ ...review, verdicts: { custom_result: "maybe" } })
      .success,
    false,
  );
  const hook = {
    kind: "project_hook",
    hook_version_id: ids.hook,
    outcomes: { passed: "ok", failed: "rework", timed_out: "ask", skipped: "not_applicable" },
    future_action: true,
  };
  assert.deepEqual(SystemStageActionSchema.parse(hook), hook);
  assert.equal(
    SystemStageActionSchema.safeParse({ ...hook, kind: "run_all_tests" }).success,
    false,
  );
  assert.equal(
    SystemStageActionSchema.safeParse({
      kind: "git_integration",
      outcomes: { applied: "ok", no_changes: "same", stale_base: "work" },
    }).success,
    false,
  );
});

test("additional Pipeline, stage, policy and transition fields survive parsing", () => {
  const raw = {
    ...deliveryPipelineFixture,
    future_pipeline: true,
    stages: deliveryPipelineFixture.stages.map((stage) => ({
      ...stage,
      future_stage: "custom",
      acceptance_policy: stage.acceptance_policy
        ? { ...stage.acceptance_policy, future_policy: 3 }
        : null,
    })),
    transitions: deliveryPipelineFixture.transitions.map((transition) => ({
      ...transition,
      future_transition: true,
      target: { ...transition.target, future_target: true },
    })),
  };
  assert.deepEqual(PipelineVersionViewSchema.parse(raw), raw);
});

test("Pipeline pages accept multiple graphs and omitted terminal cursor, not null", () => {
  for (const page of [
    { items: [deliveryPipelineFixture, analysisPipelineFixture], next_cursor: "opaque=" },
    { items: [] },
  ]) {
    assert.deepEqual(PipelineVersionListResponseSchema.parse(page), page);
  }
  assert.equal(
    PipelineVersionListResponseSchema.safeParse({ items: [], next_cursor: null }).success,
    false,
  );
  assert.equal(PipelineVersionListResponseSchema.safeParse({ items: [null] }).success, false);
});
