import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "../contracts/fixtures.ts";
import { createLiveApi } from "./api.ts";
const attempt = Object.freeze({
  taskId: ids.task,
  key: "10000000-0000-4000-8000-000000000001",
  body: JSON.stringify({
    project_id: ids.project,
    expected_revision: 8,
    payload: {
      task_id: ids.task,
      expected_task_revision: 5,
      cancellation_reason_key: "unspecified",
    },
  }),
});
const receipt = {
  command_id: ids.actor,
  status: "applied",
  project_revision: 10,
  event_ids: [ids.artifact],
  resource: { kind: "task", id: ids.task },
};
const signal = new AbortController().signal;
test("cancellation retries identical bytes/key and accepts dependency-related revision increments", async () => {
  const calls: RequestInit[] = [];
  const api = createLiveApi(async (path, init) => {
    assert.equal(path, "/api/commands/cancel_task");
    calls.push(init ?? {});
    if (calls.length === 1) throw new Error("private");
    return Response.json({ ...receipt, status: "replayed" });
  });
  await assert.rejects(api.cancelTask(attempt, "token", signal), { kind: "outcome_unknown" });
  assert.equal(calls.length, 1);
  assert.equal((await api.cancelTask(attempt, "token", signal)).project_revision, 10);
  for (const call of calls) {
    assert.equal(call.body, attempt.body);
    assert.equal(new Headers(call.headers).get("Idempotency-Key"), attempt.key);
    assert.equal(call.credentials, "omit");
    assert.equal(call.redirect, "error");
  }
});
test("cancellation validates receipt identity, safe increasing revision and bounded response", async () => {
  for (const value of [
    { ...receipt, project_revision: 8 },
    { ...receipt, project_revision: Number.MAX_SAFE_INTEGER + 1 },
    { ...receipt, resource: { kind: "task", id: ids.actor } },
    { ...receipt, event_ids: [] },
    { ...receipt, extra: "x".repeat(66000) },
  ])
    await assert.rejects(
      createLiveApi(async () => Response.json(value)).cancelTask(attempt, "token", signal),
      { kind: "outcome_unknown" },
    );
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json(receipt);
  });
  await assert.rejects(api.cancelTask({ ...attempt, taskId: ids.actor }, "token", signal), {
    kind: "invalid_request",
  });
  await assert.rejects(
    api.cancelTask(
      { ...attempt, body: JSON.stringify({ ...JSON.parse(attempt.body), actor: "injection" }) },
      "token",
      signal,
    ),
  );
  assert.equal(calls, 0);
});
test("cancellation catalog is project scoped and refuses unsupported data", async () => {
  const catalog = {
    project_id: ids.project,
    project_revision: 8,
    reasons: [{ id: "unspecified", display_name: "Unspecified", retired: false }],
  };
  const api = createLiveApi(async (path) => {
    assert.equal(path, `/api/projects/${ids.project}/cancellation-reasons`);
    return Response.json(catalog);
  });
  assert.deepEqual(await api.cancellationReasons(ids.project, "token", signal), catalog);
  await assert.rejects(
    createLiveApi(async () =>
      Response.json({ ...catalog, project_id: ids.actor }),
    ).cancellationReasons(ids.project, "token", signal),
    { kind: "invalid_response" },
  );
});
