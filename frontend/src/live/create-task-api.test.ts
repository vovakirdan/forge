import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "../contracts/fixtures.ts";
import { createLiveApi } from "./api.ts";

const attempt = Object.freeze({
  key: "10000000-0000-4000-8000-000000000001",
  body: JSON.stringify({
    project_id: ids.project,
    expected_revision: 8,
    payload: {
      title: "Draft",
      description: "",
      definition_of_done: null,
      kind: "analysis",
      priority: "normal",
      pipeline_version_id: ids.version1,
      properties: {},
    },
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
test("create accepts a Core-assigned Task identity and replays exact bytes and key", async () => {
  const calls: RequestInit[] = [];
  const api = createLiveApi(async (url, init) => {
    assert.equal(url, "/api/commands/create_task");
    calls.push(init ?? {});
    if (calls.length === 1) throw new Error("private transport detail");
    return Response.json({ ...receipt, status: "replayed" });
  });
  await assert.rejects(api.createTask(attempt, "token", signal), { kind: "outcome_unknown" });
  assert.equal(calls.length, 1);
  assert.deepEqual(await api.createTask(attempt, "token", signal), {
    ...receipt,
    status: "replayed",
  });
  for (const call of calls) {
    assert.equal(call.method, "POST");
    assert.equal(call.body, attempt.body);
    assert.equal(call.credentials, "omit");
    assert.equal(call.redirect, "error");
    assert.equal(call.cache, "no-store");
    assert.equal(new Headers(call.headers).get("Idempotency-Key"), attempt.key);
  }
});
test("creation rejects invalid identities, revisions, resource kinds and unbounded receipts", async () => {
  for (const invalid of [
    { ...receipt, resource: { kind: "task", id: "wrong" } },
    { ...receipt, resource: { kind: "project", id: ids.project } },
    { ...receipt, project_revision: 10 },
    { ...receipt, event_ids: [] },
    { ...receipt, extra: "x".repeat(66_000) },
  ])
    await assert.rejects(
      createLiveApi(async () => Response.json(invalid)).createTask(attempt, "token", signal),
      { kind: "outcome_unknown" },
    );
});
test("creation classifies only agreed refusals, never uncertain outcomes as safe to recreate", async () => {
  for (const [status, code, kind] of [
    [409, "stale_revision", "stale_revision"],
    [422, "validation_failed", "validation_failed"],
    [403, "forbidden", "forbidden"],
    [404, "not_found", "not_found"],
    [401, "anything", "unauthorized"],
    [500, "invalid_request", "outcome_unknown"],
    [409, "other", "outcome_unknown"],
  ] as const)
    await assert.rejects(
      createLiveApi(async () => Response.json({ error: "private", code }, { status })).createTask(
        attempt,
        "token",
        signal,
      ),
      { kind },
    );
});
test("invalid create payload never reaches transport", async () => {
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json(receipt);
  });
  await assert.rejects(
    api.createTask(
      {
        ...attempt,
        body: JSON.stringify({
          project_id: ids.project,
          expected_revision: 8,
          payload: { task_id: ids.task },
        }),
      },
      "token",
      signal,
    ),
  );
  assert.equal(calls, 0);
});
