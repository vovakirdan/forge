import assert from "node:assert/strict";
import { test } from "node:test";
import { ids } from "../contracts/fixtures.ts";
import { createLiveApi } from "./api.ts";
import type { DependencyDirection } from "../contracts/task-dependencies.ts";

const item = {
  blocker_task_id: ids.actor,
  blocked_task_id: ids.task,
  required_condition: "task_done",
  related_task: { id: ids.actor, key: "TASK-002", title: "Related", lifecycle: "done" },
  condition_state: "satisfied",
};
const signal = new AbortController().signal;
test("dependency GETs bind direction and encode cursor once using scoped transport", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const api = createLiveApi(async (url, init) => {
    calls.push({ url: String(url), init: init ?? {} });
    return Response.json({
      items: String(url).includes("/blocks?")
        ? [{ ...item, blocker_task_id: ids.task, blocked_task_id: ids.actor }]
        : [item],
    });
  });
  await api.taskDependencies(ids.project, ids.task, "blocked_by", "opaque+/=", "token", signal);
  await api.taskDependencies(ids.project, ids.task, "blocks", null, "token", signal);
  assert.deepEqual(
    calls.map(({ url }) => url),
    [
      `/api/projects/${ids.project}/tasks/${ids.task}/dependencies/blocked_by?limit=20&cursor=opaque%2B%2F%3D`,
      `/api/projects/${ids.project}/tasks/${ids.task}/dependencies/blocks?limit=20`,
    ],
  );
  for (const { init } of calls) {
    assert.equal(init.method, "GET");
    assert.equal(init.credentials, "omit");
    assert.equal(init.redirect, "error");
    assert.equal(init.cache, "no-store");
    assert.equal(init.signal, signal);
  }
});
test("dependency response identity, direction, duplicates and page size fail closed", async () => {
  for (const items of [
    [{ ...item, blocked_task_id: ids.project }],
    [{ ...item, related_task: { ...item.related_task, id: ids.project } }],
    [{ ...item, blocker_task_id: ids.task, related_task: { ...item.related_task, id: ids.task } }],
    [item, item],
    Array.from({ length: 21 }, () => item),
  ]) {
    const api = createLiveApi(async () => Response.json({ items }));
    await assert.rejects(
      api.taskDependencies(ids.project, ids.task, "blocked_by", null, "token", signal),
      { kind: "invalid_response" },
    );
  }
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json({ items: [] });
  });
  await assert.rejects(
    api.taskDependencies(ids.project, "bad", "blocked_by", null, "token", signal),
    { kind: "invalid_id" },
  );
  await assert.rejects(
    api.taskDependencies(
      ids.project,
      ids.task,
      "../private" as DependencyDirection,
      null,
      "token",
      signal,
    ),
    { kind: "invalid_id" },
  );
  assert.equal(calls, 0);
});
test("dependency cursors and unauthorized failures keep existing error semantics", async () => {
  for (const [status, code, kind] of [
    [409, "cursor_invalid", "cursor_invalid"],
    [401, "anything", "unauthorized"],
    [404, "not_found", "not_found"],
  ] as const) {
    const api = createLiveApi(async () =>
      Response.json({ error: "private details", code }, { status }),
    );
    await assert.rejects(
      api.taskDependencies(ids.project, ids.task, "blocked_by", null, "token", signal),
      { kind },
    );
  }
});
