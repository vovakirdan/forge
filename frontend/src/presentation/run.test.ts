import assert from "node:assert/strict";
import { test } from "node:test";
import {
  RunDetailViewSchema,
  RunViewSchema,
  TaskSummaryViewSchema,
  type RunDesiredState,
  type RunDiagnostics,
  type RunObservedState,
} from "../contracts/index.ts";
import { ids, taskSummaryFixture } from "../contracts/fixtures.ts";
import {
  assignmentFixtures,
  emptyRunDiagnosticsFixture,
  runViewFixture,
} from "../contracts/run-fixtures.ts";
import { presentRun, presentTaskSummary } from "./index.ts";
import { freezeFixture } from "./test-helpers.ts";

test("Run keeps every desired and observed state separate", () => {
  const desiredStates: [RunDesiredState, string][] = [
    ["provision_requested", "Provision requested"],
    ["running", "Running"],
    ["stop_requested", "Stop requested"],
    ["force_stop_requested", "Force stop requested"],
    ["stopped", "Stopped"],
    ["failed", "Failed"],
  ];
  const observedStates: [RunObservedState, string][] = [
    ["unknown", "Unknown"],
    ["provisioning", "Provisioning"],
    ["running", "Running"],
    ["stopping", "Stopping"],
    ["stopped", "Stopped"],
    ["failed", "Failed"],
    ["lost", "Lost"],
  ];
  for (const [desired_state, desiredStateLabel] of desiredStates) {
    for (const [observed_state, observedStateLabel] of observedStates) {
      const run = RunViewSchema.parse({ ...runViewFixture, desired_state, observed_state });
      const result = presentRun(run);
      assert.strictEqual(result.source, run);
      assert.equal(result.presentation.desiredStateLabel, desiredStateLabel);
      assert.equal(result.presentation.observedStateLabel, observedStateLabel);
    }
  }
});

test("All five Run purposes and both SystemJob kinds have labels without copying owner union", () => {
  const expected = [
    ["task_stage", "Task stage", null, 6],
    ["communication", "Communication", null, 3],
    ["resolution", "Resolution", null, 4],
    ["hook", "Hook", null, 5],
    ["system_job", "System job", "Summarization", 7],
    ["system_job", "System job", "Onboarding", 7],
  ] as const;
  for (const [
    index,
    [purpose, purposeLabel, systemJobKindLabel, run_spec_version],
  ] of expected.entries()) {
    const run = RunViewSchema.parse({
      ...runViewFixture,
      assignment: assignmentFixtures[index],
      task_id: purpose === "task_stage" ? runViewFixture.task_id : null,
      stage_id: purpose === "task_stage" ? "work" : null,
      employee_id:
        purpose === "hook" || purpose === "system_job" ? null : runViewFixture.employee_id,
      run_spec_version,
    });
    const result = presentRun(run);
    assert.equal(result.source.assignment.purpose, purpose);
    assert.strictEqual(result.source.assignment, run.assignment);
    assert.deepEqual(result.presentation, {
      purposeLabel,
      desiredStateLabel: "Stop requested",
      observedStateLabel: "Running",
      systemJobKindLabel,
    });
    if (result.source.assignment.purpose === "hook") {
      assert.equal(result.source.task_id, null);
      assert.equal(result.source.assignment.owner.task_id, runViewFixture.task_id);
    }
  }
});

test("Waiting Task and multiple simultaneous Runs of the same Employee remain independent", () => {
  const task = TaskSummaryViewSchema.parse({
    ...taskSummaryFixture,
    kind: "delivery",
    current_stage_id: "work",
    pipeline_version_id: ids.version1,
  });
  const running = RunViewSchema.parse(runViewFixture);
  const stopping = RunViewSchema.parse({
    ...runViewFixture,
    id: "01a00000-0000-7000-8000-000000000099",
    observed_state: "stopping",
    assignment: assignmentFixtures[1],
    task_id: null,
    stage_id: null,
    run_spec_version: 3,
  });
  assert.equal(running.employee_id, stopping.employee_id);
  assert.equal(presentTaskSummary(task).presentation.lifecycleLabel, "Waiting");
  assert.equal(presentRun(running).presentation.observedStateLabel, "Running");
  assert.equal(presentRun(stopping).presentation.observedStateLabel, "Stopping");
  assert.strictEqual(presentRun(running).source, running);
  assert.strictEqual(presentRun(stopping).source, stopping);
});

test("Run detail retains its typed diagnostics through the same Run presenter", () => {
  const run = RunDetailViewSchema.parse({
    ...runViewFixture,
    diagnostics: emptyRunDiagnosticsFixture,
  });
  const result = presentRun(run);
  const diagnostics: RunDiagnostics = result.source.diagnostics;
  assert.strictEqual(result.source, run);
  assert.strictEqual(diagnostics, run.diagnostics);
});

test("Run presentation preserves source extensions and never mutates frozen input", () => {
  const raw = JSON.parse(JSON.stringify(runViewFixture));
  const extensions = JSON.parse('{"__proto__":{"retained":true},"constructor":{"opaque":true}}');
  raw.extra = extensions;
  raw.assignment.owner.extra = extensions;
  const run = freezeFixture(RunViewSchema.parse(raw));
  const before = JSON.stringify(run);
  const result = presentRun(run);
  assert.strictEqual(result.source, run);
  assert.strictEqual(result.source.assignment.owner["extra"], extensions);
  assert.deepEqual(result, presentRun(run));
  assert.equal(JSON.stringify(run), before);
});
