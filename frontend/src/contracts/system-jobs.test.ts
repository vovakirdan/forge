import assert from "node:assert/strict";
import { test } from "node:test";
import {
  EmployeeOnboardingStatusSchema,
  SystemJobAttemptPageSchema,
  SystemJobStatusSchema,
} from "./system-jobs.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const job = "01988000-0000-7000-8000-000000000003";

test("SystemJob status keeps policy, job generation, usage and onboarding separate", () => {
  const status = SystemJobStatusSchema.parse({
    policy: null,
    settings_revision: null,
    jobs: [
      {
        id: job,
        project_id: project,
        kind: "onboarding",
        source_task_id: null,
        target_employee_id: employee,
        generation: 2,
        covered_sequence: 5,
        state: "held",
        reason_code: "admission_closed",
      },
    ],
    usage: {
      attempts_last_24h: 1,
      running: 0,
      usage_status: "see_per_run_accounting_unknown_is_not_zero",
    },
    onboarding: [
      { employee_id: employee, state: "pending", revision: 1, job_id: job, receipt: null },
    ],
  });
  assert.equal(status.jobs[0]?.state, "held");
  assert.equal(status.onboarding[0]?.state, "pending");
  assert.equal(status.usage.running, 0);
  assert.equal(status.policy, null);
});

test("legacy bypass and skipped onboarding remain distinct from completed receipt", () => {
  for (const state of ["legacy_bypass", "skipped", "completed"] as const) {
    const value = EmployeeOnboardingStatusSchema.parse({
      employee_id: employee,
      state,
      revision: 2,
      job_id: null,
      receipt: state === "completed" ? { source_digest: "digest" } : null,
    });
    assert.equal(value.state, state);
  }
  assert.equal(
    EmployeeOnboardingStatusSchema.safeParse({
      employee_id: employee,
      state: "failed",
      revision: 1,
      job_id: null,
      receipt: null,
    }).success,
    false,
  );
});

test("attempt projection separates accepted result from materialized coordinates", () => {
  const attempt = {
    id: "01988000-0000-7000-8000-000000000004",
    job_id: job,
    generation: 2,
    run_id: "01988000-0000-7000-8000-000000000005",
    state: "superseded",
    result_accepted: true,
    result_message_id: "01988000-0000-7000-8000-000000000006",
    created_at: "2026-09-23T08:00:00Z",
    completed_at: "2026-09-23T08:01:00Z",
    materialized_entries: [],
  };
  assert.equal(
    SystemJobAttemptPageSchema.parse({ items: [attempt] }).items[0]?.result_accepted,
    true,
  );
  for (const unsafe of [
    { ...attempt, spec: { binding: { secret: "private" } } },
    { ...attempt, result: { markdown: "private" } },
    { ...attempt, result_accepted: false },
    { ...attempt, materialized_entries: [{ id: employee, revision: 1, markdown: "private" }] },
  ]) {
    assert.equal(SystemJobAttemptPageSchema.safeParse({ items: [unsafe] }).success, false);
  }
});
