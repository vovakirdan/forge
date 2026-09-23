import { test } from "node:test";
import assert from "node:assert/strict";
import { managerPlanningAttempt } from "./manager-planning.ts";

const project = "01988000-0000-7000-8000-000000000001";
const task = "01988000-0000-7000-8000-000000000002";
const employee = "01988000-0000-7000-8000-000000000003";
test("next-Run assignment is scoped to one Task revision", () => {
  const attempt = managerPlanningAttempt("set_next_run_employee", {
    project_id: project,
    expected_revision: 3,
    payload: { task_id: task, expected_task_revision: 2, employee_id: employee, reason: null },
  });
  assert.equal(attempt.resourceId, task);
  assert.throws(() =>
    managerPlanningAttempt("set_next_run_employee", {
      project_id: project,
      expected_revision: 3,
      payload: { task_id: task, expected_task_revision: 0, employee_id: employee, reason: null },
    }),
  );
  assert.throws(() =>
    managerPlanningAttempt("set_next_run_employee", {
      project_id: project,
      expected_revision: 3,
      payload: {
        task_id: task,
        expected_task_revision: 2,
        employee_id: employee,
        stop_current: true,
        reason: null,
      },
    }),
  );
});
test("resume scheduling requires an exact wait, time and audit reason", () => {
  const payload = {
    task_id: task,
    expected_task_revision: 2,
    wait_condition_id: employee,
    not_before: "2030-01-01T00:00:00Z",
    reason: "After maintenance",
  };
  assert.equal(
    managerPlanningAttempt("schedule_task_resume", {
      project_id: project,
      expected_revision: 3,
      payload,
    }).resourceId,
    null,
  );
  assert.throws(() =>
    managerPlanningAttempt("schedule_task_resume", {
      project_id: project,
      expected_revision: 3,
      payload: { ...payload, reason: " " },
    }),
  );
});
