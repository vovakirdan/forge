import assert from "node:assert/strict";
import { test } from "node:test";
import { AmendEmployeeRequestSchema, amendEmployeeAttempt } from "./amend-employee.ts";

const request = {
  project_id: "01988000-0000-7000-8000-000000000001",
  expected_revision: 3,
  payload: {
    employee_id: "01988000-0000-7000-8000-000000000002",
    expected_employee_revision: 2,
    patch: { name: "New name", max_concurrent_runs: 2 },
  },
};

test("Employee amendment uses a strict nonempty patch and frozen retry body", () => {
  const parsed = AmendEmployeeRequestSchema.parse(request);
  const attempt = amendEmployeeAttempt(parsed, "11111111-1111-4111-8111-111111111111");
  assert.equal(Object.isFrozen(attempt), true);
  assert.deepEqual(JSON.parse(attempt.body).payload.patch, request.payload.patch);
  for (const patch of [
    {},
    { name: " " },
    { max_concurrent_runs: 0 },
    { max_concurrent_runs: 65536 },
    { state: "disabled" },
    { stage_eligibility: { mode: "only", stages: [] } },
  ])
    assert.equal(
      AmendEmployeeRequestSchema.safeParse({ ...request, payload: { ...request.payload, patch } })
        .success,
      false,
    );
});
