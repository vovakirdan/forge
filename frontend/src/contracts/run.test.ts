import assert from "node:assert/strict";
import { test } from "node:test";
import {
  RunDesiredStateSchema,
  RunDetailViewSchema,
  RunListResponseSchema,
  RunObservedStateSchema,
  RunViewSchema,
  TaskSummaryViewSchema,
} from "./index.ts";
import { ids, taskSummaryFixture } from "./fixtures.ts";
import { assignmentFixtures, emptyRunDiagnosticsFixture, runViewFixture } from "./run-fixtures.ts";

test("Run desired and observed vocabularies remain separate and preserve mismatched observations", () => {
  const desiredStates = [
    "provision_requested",
    "running",
    "stop_requested",
    "force_stop_requested",
    "stopped",
    "failed",
  ];
  const observedStates = [
    "unknown",
    "provisioning",
    "running",
    "stopping",
    "stopped",
    "failed",
    "lost",
  ];
  assert.deepEqual(RunDesiredStateSchema.options, desiredStates);
  assert.deepEqual(RunObservedStateSchema.options, observedStates);
  for (const desired_state of desiredStates) {
    assert.equal(
      RunViewSchema.parse({ ...runViewFixture, desired_state }).desired_state,
      desired_state,
    );
  }
  for (const observed_state of observedStates) {
    assert.equal(
      RunViewSchema.parse({ ...runViewFixture, observed_state }).observed_state,
      observed_state,
    );
  }
  for (const patch of [
    { desired_state: "stop_requested", observed_state: "running" },
    { desired_state: "force_stop_requested", observed_state: "unknown" },
  ]) {
    const raw = { ...runViewFixture, ...patch };
    assert.deepEqual(RunViewSchema.parse(raw), raw);
  }
  for (const patch of [
    { desired_state: "stopping" },
    { observed_state: "stop_requested" },
    { desired_state: "done" },
    { observed_state: "paused" },
  ]) {
    assert.equal(RunViewSchema.safeParse({ ...runViewFixture, ...patch }).success, false);
  }
});

test("Run handles taskless communication/resolution and ownerless hooks/SystemJobs without fabricated IDs", () => {
  const currentVersions: Record<string, number> = {
    task_stage: 6,
    communication: 3,
    resolution: 4,
    hook: 5,
    system_job: 7,
  };
  for (const assignment of assignmentFixtures) {
    const runSpecVersion = currentVersions[assignment.purpose];
    assert.ok(runSpecVersion);
    const taskStage = assignment.purpose === "task_stage";
    const noEmployee = ["hook", "system_job"].includes(assignment.purpose);
    const raw = {
      ...runViewFixture,
      assignment,
      task_id: taskStage ? ids.task : null,
      stage_id: taskStage ? "work" : null,
      employee_id: noEmployee ? null : runViewFixture.employee_id,
      run_spec_version: runSpecVersion,
    };
    const run = RunViewSchema.parse(raw);
    assert.deepEqual(run, raw);
    if (run.assignment.purpose === "hook") {
      assert.equal(run.assignment.owner.task_id, ids.task);
      assert.equal(run.task_id, null);
      assert.equal(run.stage_id, null);
    }
    for (const absent of [
      "project_id",
      "provider",
      "model",
      "created_at",
      "work_surface",
      "employee",
    ]) {
      assert.equal(Object.hasOwn(run, absent), false);
    }
  }
});

test("one Employee may have multiple Runs and waiting Task does not imply Run quiescence", () => {
  const task = TaskSummaryViewSchema.parse({
    ...taskSummaryFixture,
    current_stage_id: "work",
    kind: "delivery",
    pipeline_version_id: ids.version1,
  });
  const secondRun = {
    ...runViewFixture,
    id: "01a00000-0000-7000-8000-000000000031",
    assignment: assignmentFixtures[1],
    run_spec_version: 3,
    task_id: null,
    stage_id: null,
    observed_state: "stopping",
  };
  const page = RunListResponseSchema.parse({ items: [runViewFixture, secondRun] });
  assert.equal(task.lifecycle, "waiting");
  assert.deepEqual(
    page.items.map((run) => run.observed_state),
    ["running", "stopping"],
  );
  assert.equal(page.items[0]?.employee_id, page.items[1]?.employee_id);
});

test("Run requires all wire fields and distinguishes explicit null from omission", () => {
  for (const field of Object.keys(runViewFixture)) {
    const raw: Record<string, unknown> = { ...runViewFixture };
    delete raw[field];
    assert.equal(RunViewSchema.safeParse(raw).success, false, field);
  }
  for (const field of ["task_id", "employee_id", "stage_id"]) {
    assert.equal(RunViewSchema.safeParse({ ...runViewFixture, [field]: null }).success, true);
    assert.equal(RunViewSchema.safeParse({ ...runViewFixture, [field]: 1 }).success, false);
  }
  assert.equal(RunViewSchema.safeParse({ ...runViewFixture, id: "not-an-id" }).success, false);
  assert.equal(
    RunViewSchema.safeParse({ ...runViewFixture, stage_id: "Invalid stage" }).success,
    false,
  );
});

test("Run numeric fields enforce wire widths, positivity and JavaScript exactness", () => {
  const bounds = [
    ["attempt", 1, 4_294_967_295],
    ["run_spec_version", 1, 65_535],
    ["lease_fencing_token", 1, Number.MAX_SAFE_INTEGER],
    ["environment_epoch", 1, Number.MAX_SAFE_INTEGER],
    ["last_observed_sequence", 0, Number.MAX_SAFE_INTEGER],
  ] as const;
  for (const [field, minimum, maximum] of bounds) {
    for (const value of [minimum, maximum]) {
      assert.equal(
        RunViewSchema.safeParse({ ...runViewFixture, [field]: value }).success,
        true,
        field,
      );
    }
    for (const value of [minimum - 1, maximum + 1, -1, 1.5, "1", null, Infinity, NaN]) {
      assert.equal(
        RunViewSchema.safeParse({ ...runViewFixture, [field]: value }).success,
        false,
        field,
      );
    }
  }
  assert.equal(
    RunViewSchema.parse({ ...runViewFixture, run_spec_version: 42 }).run_spec_version,
    42,
  );
});

test("Run detail flattens Run fields and requires its diagnostics envelope", () => {
  const raw = { ...runViewFixture, diagnostics: emptyRunDiagnosticsFixture };
  assert.deepEqual(RunDetailViewSchema.parse(raw), raw);
  assert.equal(RunDetailViewSchema.safeParse(runViewFixture).success, false);
  assert.equal(
    RunDetailViewSchema.safeParse({ run: runViewFixture, diagnostics: emptyRunDiagnosticsFixture })
      .success,
    false,
  );
  assert.equal(RunDetailViewSchema.safeParse({ ...raw, diagnostics: null }).success, false);
  assert.equal(
    RunDetailViewSchema.safeParse({ ...raw, object_key: "private/object" }).success,
    false,
  );
});

test("Run pagination retains opaque cursors and rejects unexpected fields", () => {
  const extra: Record<string, unknown> = JSON.parse(
    '{"__proto__":{"data":true},"constructor":null}',
  );
  const item = { ...runViewFixture, ...extra };
  assert.equal(RunListResponseSchema.safeParse({ items: [item] }).success, false);
  assert.equal(RunListResponseSchema.safeParse({ items: [], ...extra }).success, false);
  assert.deepEqual(RunListResponseSchema.parse({ items: [], next_cursor: "opaque/+cursor=" }), {
    items: [],
    next_cursor: "opaque/+cursor=",
  });
  assert.equal(Object.hasOwn(RunListResponseSchema.parse({ items: [] }), "next_cursor"), false);
  for (const invalid of [
    {},
    { items: null },
    { items: [null] },
    { items: [], next_cursor: null },
    { items: [], next_cursor: 1 },
  ]) {
    assert.equal(RunListResponseSchema.safeParse(invalid).success, false);
  }
});
