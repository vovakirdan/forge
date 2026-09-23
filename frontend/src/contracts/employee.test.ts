import assert from "node:assert/strict";
import test from "node:test";
import { EmployeeProfileSchema } from "./employee.ts";

const employee = "01988000-0000-7000-8000-000000000001";
const project = "01988000-0000-7000-8000-000000000002";
const pipeline = "01988000-0000-7000-8000-000000000003";
const base = {
  id: employee,
  project_id: project,
  name: "Worker",
  role: "reviewer",
  state: "enabled",
  revision: 2,
  max_concurrent_runs: 1,
  created_at: "2026-09-23T00:00:00Z",
  updated_at: "2026-09-23T01:00:00Z",
};

test("Employee profile validates both stage eligibility modes without adding availability", () => {
  const any = { ...base, stage_eligibility: { mode: "any" }, future: { added: true } };
  assert.strictEqual(EmployeeProfileSchema.parse(any), any);
  const only = {
    ...base,
    stage_eligibility: {
      mode: "only",
      stages: [{ pipeline_version_id: pipeline, stage_id: "review" }],
    },
  };
  assert.strictEqual(EmployeeProfileSchema.parse(only), only);
  assert.equal(
    EmployeeProfileSchema.safeParse({ ...base, stage_eligibility: { mode: "only", stages: [] } })
      .success,
    false,
  );
  assert.equal(
    EmployeeProfileSchema.safeParse({ ...base, stage_eligibility: { mode: "busy" } }).success,
    false,
  );
});
