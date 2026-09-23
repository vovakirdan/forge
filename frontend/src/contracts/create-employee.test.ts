import assert from "node:assert/strict";
import { test } from "node:test";
import { CreateEmployeeRequestSchema, createEmployeeAttempt } from "./create-employee.ts";

const project = "01988000-0000-7000-8000-000000000001";
const version = "01988000-0000-7000-8000-000000000002";
const request = {
  project_id: project,
  expected_revision: 3,
  payload: { name: "New teammate", role: "reviewer", stage_eligibility: { mode: "any" } },
};

test("Employee creation requires explicit eligibility and retains one immutable retry body", () => {
  assert.equal(CreateEmployeeRequestSchema.safeParse(request).success, true);
  const attempt = createEmployeeAttempt(
    CreateEmployeeRequestSchema.parse(request),
    "11111111-1111-4111-8111-111111111111",
  );
  assert.equal(JSON.parse(attempt.body).payload.stage_eligibility.mode, "any");
  assert.equal(Object.isFrozen(attempt), true);
  assert.equal(
    CreateEmployeeRequestSchema.safeParse({
      ...request,
      payload: { ...request.payload, stage_eligibility: undefined },
    }).success,
    false,
  );
  assert.equal(
    CreateEmployeeRequestSchema.safeParse({ ...request, actor: "owner" }).success,
    false,
  );
  assert.equal(
    CreateEmployeeRequestSchema.safeParse({
      ...request,
      payload: { ...request.payload, token: "secret" },
    }).success,
    false,
  );
});

test("Only mode rejects duplicate, unscoped and empty stage targets", () => {
  const stage = { pipeline_version_id: version, stage_id: "work" };
  const candidate = (stages: unknown) => ({
    ...request,
    payload: { ...request.payload, stage_eligibility: { mode: "only", stages } },
  });
  assert.equal(CreateEmployeeRequestSchema.safeParse(candidate([stage])).success, true);
  for (const stages of [
    [],
    [stage, stage],
    [{ stage_id: "work" }],
    [{ ...stage, stage_id: "Work" }],
  ])
    assert.equal(CreateEmployeeRequestSchema.safeParse(candidate(stages)).success, false);
});
