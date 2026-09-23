import assert from "node:assert/strict";
import { test } from "node:test";
import { managementAttempt } from "./management-command.ts";
import {
  EscalationListSchema,
  RecoveryReadinessSchema,
  ResumeScheduleListSchema,
} from "./management.ts";
const project = "01988000-0000-7000-8000-000000000001";
const task = "01988000-0000-7000-8000-000000000002";
const wait = "01988000-0000-7000-8000-000000000003";
const key = "550e8400-e29b-41d4-a716-446655440000";
test("management attempts freeze exact revision and selected wait", () => {
  const attempt = managementAttempt(
    "resume_task",
    {
      project_id: project,
      expected_revision: 9,
      payload: { task_id: task, expected_task_revision: 3, wait_condition_id: wait },
    },
    key,
  );
  assert.equal(attempt.resourceId, task);
  assert.equal(JSON.parse(attempt.body).payload.wait_condition_id, wait);
  assert.ok(Object.isFrozen(attempt));
  assert.throws(() =>
    managementAttempt(
      "pause_task",
      {
        project_id: project,
        expected_revision: 9,
        payload: { task_id: task, expected_task_revision: 3, mode: "instant", reason: null },
      },
      key,
    ),
  );
});
test("management read contracts keep scheduled intent separate from resolution", () => {
  const schedules = ResumeScheduleListSchema.parse({
    items: [
      {
        id: wait,
        task_id: task,
        expected_task_revision: 3,
        pipeline_version_id: project,
        stage_id: "build",
        stage_visit: 1,
        wait_condition_id: wait,
        not_before: "2026-09-23T10:00:00Z",
        reason: "operator",
        created_at: "2026-09-23T09:00:00Z",
        state: { status: "pending" },
      },
    ],
  });
  assert.equal(schedules.items[0]?.state.status, "pending");
  const escalations = EscalationListSchema.parse({
    items: [
      {
        id: wait,
        source: {
          kind: "communication",
          run_id: task,
          assignment_id: task,
          thread_id: task,
          source_message_id: task,
        },
        category: "clarification",
        question: "Need clarification",
        allowed_outcomes: [],
        revision: 1,
        generation: 0,
        state: { status: "queued" },
        latest_assignment: null,
        created_at: "2026-09-23T09:00:00Z",
      },
    ],
  });
  assert.equal(escalations.items[0]?.source.kind, "communication");
  assert.deepEqual(escalations.items[0]?.allowed_outcomes, []);
});

test("human resolution freezes exact assignment fence and allowed answer", () => {
  const attempt = managementAttempt(
    "submit_human_resolution",
    {
      project_id: project,
      expected_revision: 7,
      payload: {
        escalation_id: task,
        expected_escalation_revision: 2,
        assignment_id: wait,
        lease_generation: 3,
        answer: {
          disposition: "needs_management_change",
          summary: "Policy review",
          recommended_outcome_key: null,
        },
      },
    },
    key,
  );
  assert.equal(attempt.resourceId, task);
  assert.equal(JSON.parse(attempt.body).payload.lease_generation, 3);
  assert.throws(() =>
    managementAttempt(
      "submit_human_resolution",
      {
        project_id: project,
        expected_revision: 7,
        payload: {
          escalation_id: task,
          expected_escalation_revision: 2,
          assignment_id: wait,
          lease_generation: 0,
          answer: { disposition: "continue_stage", summary: "yes", recommended_outcome_key: null },
        },
      },
      key,
    ),
  );
});

test("recovery acceptance only prepares the exact confirmed non-start assessment", () => {
  const readiness = RecoveryReadinessSchema.parse({
    run_id: task,
    assessment: "not_started_confirmed",
    eligible: true,
    reason_code: null,
    project_revision: 11,
    run_revision: 2,
    lease_fencing_token: 3,
    environment_epoch: 4,
    task_id: wait,
    task_revision: 5,
  });
  const attempt = managementAttempt(
    "accept_run_recovery_assessment",
    {
      project_id: project,
      expected_revision: readiness.project_revision,
      payload: { run_id: readiness.run_id, assessment: readiness.assessment },
    },
    key,
  );
  assert.equal(attempt.resourceId, task);
  assert.equal(JSON.parse(attempt.body).payload.assessment, "not_started_confirmed");
  assert.throws(() =>
    managementAttempt(
      "accept_run_recovery_assessment",
      {
        project_id: project,
        expected_revision: 11,
        payload: { run_id: task, assessment: "unknown" },
      },
      key,
    ),
  );
  assert.equal(RecoveryReadinessSchema.safeParse({ ...readiness, eligible: false }).success, false);
});
