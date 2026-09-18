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
    payload: { task_id: ids.task, expected_task_revision: 5, priority: "high" },
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
test("priority retry retains the exact command, body and key without automatic retries", async () => {
  const calls: RequestInit[] = [];
  const api = createLiveApi(async (path, init) => {
    assert.equal(path, "/api/commands/set_task_priority");
    calls.push(init ?? {});
    if (calls.length === 1) throw new Error("private detail");
    return Response.json({ ...receipt, status: "replayed" });
  });
  await assert.rejects(api.setTaskPriority(attempt, "token", signal), { kind: "outcome_unknown" });
  assert.equal(calls.length, 1);
  assert.equal((await api.setTaskPriority(attempt, "token", signal)).status, "replayed");
  for (const call of calls) {
    assert.equal(call.method, "POST");
    assert.equal(call.body, attempt.body);
    assert.equal(call.credentials, "omit");
    assert.equal(call.redirect, "error");
    assert.equal(call.cache, "no-store");
    assert.equal(new Headers(call.headers).get("Idempotency-Key"), attempt.key);
  }
});
test("priority only recognizes exact refusal pairs, all other responses remain unknown", async () => {
  for (const [status, code, kind] of [
    [400, "invalid_request", "invalid_request"],
    [409, "stale_revision", "stale_revision"],
    [409, "idempotency_conflict", "idempotency_conflict"],
    [422, "validation_failed", "validation_failed"],
    [403, "forbidden", "forbidden"],
    [404, "not_found", "not_found"],
    [401, "other", "unauthorized"],
    [500, "invalid_request", "outcome_unknown"],
    [409, "other", "outcome_unknown"],
  ] as const) {
    const api = createLiveApi(async () => Response.json({ error: "private", code }, { status }));
    await assert.rejects(api.setTaskPriority(attempt, "token", signal), { kind });
  }
});
test("mismatched or oversized receipts do not prove priority saved", async () => {
  for (const value of [
    { ...receipt, project_revision: 10 },
    { ...receipt, resource: { kind: "task", id: ids.actor } },
    { ...receipt, event_ids: [] },
    { ...receipt, extra: "x".repeat(66000) },
  ]) {
    await assert.rejects(
      createLiveApi(async () => Response.json(value)).setTaskPriority(attempt, "token", signal),
      { kind: "outcome_unknown" },
    );
  }
});
test("priority validation rejects unsupported payloads before transport", async () => {
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json(receipt);
  });
  await assert.rejects(api.setTaskPriority({ ...attempt, taskId: ids.actor }, "token", signal), {
    kind: "invalid_request",
  });
  await assert.rejects(
    api.setTaskPriority({ ...attempt, body: JSON.stringify({ extra: true }) }, "token", signal),
  );
  assert.equal(calls, 0);
});
