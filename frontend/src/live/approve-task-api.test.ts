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
    payload: { task_id: ids.task, expected_task_revision: 5 },
  }),
});
const receipt = {
  command_id: ids.actor,
  status: "applied",
  project_revision: 9,
  event_ids: [ids.artifact],
  resource: { kind: "task", id: ids.task },
};
const signal = new AbortController().signal;
test("approval retries exact bytes and key only when explicitly requested", async () => {
  const calls: RequestInit[] = [];
  const api = createLiveApi(async (path, init) => {
    assert.equal(path, "/api/commands/approve_task");
    calls.push(init ?? {});
    if (calls.length === 1) throw new Error("private detail");
    return Response.json({ ...receipt, status: "replayed" });
  });
  await assert.rejects(api.approveTask(attempt, "token", signal), { kind: "outcome_unknown" });
  assert.equal(calls.length, 1);
  assert.equal((await api.approveTask(attempt, "token", signal)).status, "replayed");
  for (const call of calls) {
    assert.equal(call.body, attempt.body);
    assert.equal(call.method, "POST");
    assert.equal(call.credentials, "omit");
    assert.equal(call.redirect, "error");
    assert.equal(new Headers(call.headers).get("Idempotency-Key"), attempt.key);
  }
});
test("approval validates exact receipt and refuses injection before transport", async () => {
  for (const value of [
    { ...receipt, project_revision: 10 },
    { ...receipt, resource: { kind: "task", id: ids.actor } },
    { ...receipt, event_ids: [] },
    { ...receipt, extra: "x".repeat(66000) },
  ])
    await assert.rejects(
      createLiveApi(async () => Response.json(value)).approveTask(attempt, "token", signal),
      { kind: "outcome_unknown" },
    );
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json(receipt);
  });
  await assert.rejects(api.approveTask({ ...attempt, taskId: ids.actor }, "token", signal), {
    kind: "invalid_request",
  });
  await assert.rejects(
    api.approveTask(
      { ...attempt, body: JSON.stringify({ ...JSON.parse(attempt.body), actor: "injection" }) },
      "token",
      signal,
    ),
  );
  assert.equal(calls, 0);
});
test("approval separates known refusals from ambiguous server responses", async () => {
  for (const [status, code, kind] of [
    [409, "stale_revision", "stale_revision"],
    [422, "validation_failed", "validation_failed"],
    [400, "invalid_request", "invalid_request"],
    [401, "other", "unauthorized"],
    [500, "invalid_request", "outcome_unknown"],
  ] as const)
    await assert.rejects(
      createLiveApi(async () => Response.json({ error: "private", code }, { status })).approveTask(
        attempt,
        "token",
        signal,
      ),
      { kind },
    );
});
