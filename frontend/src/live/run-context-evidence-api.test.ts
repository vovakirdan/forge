import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi } from "./api.ts";
import { ids } from "../contracts/fixtures.ts";
import { runIds } from "../contracts/run-fixtures.ts";

const context = {
  availability: "available",
  coordinates: {
    context_snapshot_id: runIds.assignment,
    run_id: runIds.run,
    project_id: ids.project,
    task_id: ids.task,
    employee_id: runIds.employee,
    pipeline_version_id: ids.version1,
    stage_id: "work",
    stage_visit: null,
    task_revision_before_dispatch: 1,
    run_spec_id: runIds.job,
  },
};
const receipt = {
  id: runIds.message,
  run_id: runIds.run,
  task_id: ids.task,
  stream: "stdout",
  sequence: 1,
  sha256: "a".repeat(64),
  size_bytes: 40,
  redaction_policy_reference: "default",
  storage_state: "stored",
  created_at: "2026-01-01T00:00:00Z",
  content_availability: "unavailable",
};

test("Run context and evidence reads remain scoped, bounded and metadata only", async () => {
  const paths: string[] = [];
  const api = createLiveApi(async (url) => {
    const path = String(url);
    paths.push(path);
    return Response.json(path.includes("/context") ? context : { items: [receipt] });
  });
  const signal = new AbortController().signal;
  assert.deepEqual(await api.runContext(ids.project, runIds.run, "token", signal), context);
  assert.deepEqual(await api.runEvidence(ids.project, runIds.run, null, "token", signal), {
    items: [receipt],
  });
  assert.deepEqual(paths, [
    `/api/projects/${ids.project}/runs/${runIds.run}/context`,
    `/api/projects/${ids.project}/runs/${runIds.run}/evidence?limit=20`,
  ]);
});

test("Run metadata boundary rejects cross-run receipts and private object references", async () => {
  const signal = new AbortController().signal;
  const wrong = createLiveApi(async () =>
    Response.json({ items: [{ ...receipt, run_id: ids.actor }] }),
  );
  await assert.rejects(wrong.runEvidence(ids.project, runIds.run, null, "token", signal), {
    kind: "invalid_response",
  });
  const privateReference = createLiveApi(async () =>
    Response.json({ items: [{ ...receipt, object_key: "private/canary" }] }),
  );
  await assert.rejects(
    privateReference.runEvidence(ids.project, runIds.run, null, "token", signal),
    {
      kind: "invalid_response",
    },
  );
  const wrongContext = createLiveApi(async () =>
    Response.json({ ...context, coordinates: { ...context.coordinates, project_id: ids.actor } }),
  );
  await assert.rejects(wrongContext.runContext(ids.project, runIds.run, "token", signal), {
    kind: "invalid_response",
  });
});
