import assert from "node:assert/strict";
import { test } from "node:test";
import { ids, timestamp } from "../contracts/fixtures.ts";
import { createLiveApi } from "./api.ts";

const handoff = {
  id: ids.artifact,
  project_id: ids.project,
  task_id: ids.task,
  kind: "accepted",
  source_kind: "run",
  source_id: ids.actor,
  created_at: timestamp,
  content_availability: "unavailable",
};

test("Task handoff read uses one scoped bounded page", async () => {
  const calls: string[] = [];
  const api = createLiveApi(async (url) => {
    calls.push(String(url));
    return Response.json({ items: [handoff] });
  });
  assert.deepEqual(
    await api.taskHandoffs(ids.project, ids.task, null, "token", new AbortController().signal),
    { items: [handoff] },
  );
  assert.deepEqual(calls, [`/api/projects/${ids.project}/tasks/${ids.task}/handoffs?limit=20`]);
});

test("Task handoff read rejects foreign scope and body material", async () => {
  const signal = new AbortController().signal;
  for (const value of [
    { ...handoff, task_id: ids.actor },
    { ...handoff, body: { private: "canary" } },
  ]) {
    const api = createLiveApi(async () => Response.json({ items: [value] }));
    await assert.rejects(api.taskHandoffs(ids.project, ids.task, null, "token", signal), {
      kind: "invalid_response",
    });
  }
});
