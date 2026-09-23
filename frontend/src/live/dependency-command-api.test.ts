import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "../contracts/fixtures.ts";
import { dependencyAttempt } from "../contracts/dependency-command.ts";
import { createLiveApi } from "./api.ts";

const signal = new AbortController().signal;
const key = "10000000-0000-4000-8000-000000000001";
const attempt = dependencyAttempt("create_dependency", ids.project, 8, ids.actor, ids.task, key);
const receipt = {
  command_id: ids.artifact,
  status: "applied",
  project_revision: 9,
  event_ids: [ids.pipeline],
  resource: { kind: "task_dependency", id: ids.task },
};

test("dependency command keeps exact bytes and key on replay", async () => {
  const calls: RequestInit[] = [];
  const api = createLiveApi(async (path, init) => {
    assert.equal(path, "/api/commands/create_dependency");
    calls.push(init ?? {});
    if (calls.length === 1) throw new Error("private");
    return Response.json({ ...receipt, status: "replayed" });
  });
  await assert.rejects(api.dependencyCommand(attempt, "token", signal), {
    kind: "outcome_unknown",
  });
  assert.equal((await api.dependencyCommand(attempt, "token", signal)).status, "replayed");
  for (const call of calls) {
    assert.equal(call.body, attempt.body);
    assert.equal(new Headers(call.headers).get("Idempotency-Key"), key);
    assert.equal(call.credentials, "omit");
  }
});

test("dependency command accepts changed and no-op receipts but rejects mismatches", async () => {
  const remove = dependencyAttempt("remove_dependency", ids.project, 8, ids.actor, ids.task, key);
  const valid = [receipt, { ...receipt, project_revision: 8, event_ids: [] }];
  for (const value of valid)
    assert.equal(
      (
        await createLiveApi(async () => Response.json(value)).dependencyCommand(
          remove,
          "token",
          signal,
        )
      ).project_revision,
      value.project_revision,
    );
  for (const value of [
    { ...receipt, project_revision: 8 },
    { ...receipt, project_revision: 9, event_ids: [] },
    { ...receipt, resource: { kind: "task", id: ids.task } },
    { ...receipt, resource: { kind: "task_dependency", id: ids.actor } },
  ])
    await assert.rejects(
      createLiveApi(async () => Response.json(value)).dependencyCommand(remove, "token", signal),
      { kind: "outcome_unknown" },
    );
});

test("dependency attempt refuses self-links and tampered request before transport", async () => {
  assert.throws(() =>
    dependencyAttempt("create_dependency", ids.project, 8, ids.task, ids.task, key),
  );
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json(receipt);
  });
  await assert.rejects(
    api.dependencyCommand({ ...attempt, blockedId: ids.actor }, "token", signal),
    { kind: "invalid_request" },
  );
  await assert.rejects(
    api.dependencyCommand(
      { ...attempt, action: "pause_task" as typeof attempt.action },
      "token",
      signal,
    ),
    { kind: "invalid_request" },
  );
  await assert.rejects(api.dependencyCommand({ ...attempt, body: "not json" }, "token", signal), {
    kind: "invalid_request",
  });
  assert.equal(calls, 0);
});
