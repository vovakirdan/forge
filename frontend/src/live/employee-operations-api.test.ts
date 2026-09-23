import assert from "node:assert/strict";
import test from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";

test("operational read is scoped and preserves unknown availability", async () => {
  const fetcher: typeof fetch = async (input) => {
    assert.equal(String(input), `/api/projects/${project}/employees/${employee}/operations`);
    return Response.json({
      employee_id: employee,
      max_concurrent_runs: 2,
      occupied_slots: 1,
      observed_running_runs: 0,
      runtime_binding_configured: false,
      availability: "unknown",
    });
  };
  const view = await createLiveApi(fetcher).employeeOperations(
    project,
    employee,
    "token",
    new AbortController().signal,
  );
  assert.equal(view.availability, "unknown");
  assert.equal(view.occupied_slots, 1);
});

test("operational read rejects a foreign Employee identity", async () => {
  const fetcher: typeof fetch = async () =>
    Response.json({
      employee_id: project,
      max_concurrent_runs: 2,
      occupied_slots: 1,
      observed_running_runs: 0,
      runtime_binding_configured: false,
      availability: "unknown",
    });
  await assert.rejects(
    createLiveApi(fetcher).employeeOperations(
      project,
      employee,
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});
