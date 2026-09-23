import assert from "node:assert/strict";
import test from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const other = "01988000-0000-7000-8000-000000000002";
const resource = (project_id: string) => ({
  project_id,
  policy: {
    revision: 1,
    host_max_runs: 3,
    project_max_runs: 2,
    credential_account_max_runs: 1,
    updated_at: "2026-09-23T00:00:00Z",
  },
  occupancy: { host_runs: 1, project_runs: 1, credential_account_runs: null },
  observed_at: "2026-09-23T00:00:01Z",
});

test("resource read accepts only the selected Project and unknown account occupancy", async () => {
  const api = createLiveApi(async (input) => {
    assert.equal(String(input), `/api/projects/${project}/resources`);
    return Response.json(resource(project));
  });
  const view = await api.resources(project, "token", new AbortController().signal);
  assert.equal(view.occupancy.credential_account_runs, null);
  const foreign = createLiveApi(async () => Response.json(resource(other)));
  await assert.rejects(
    foreign.resources(project, "token", new AbortController().signal),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});
