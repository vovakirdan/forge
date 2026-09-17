import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, describeApiError } from "./api.ts";
import {
  ids,
  taskSummaryFixture,
  taskDetailFixture,
  deliveryPipelineFixture,
} from "../contracts/fixtures.ts";

test("Task reads use fixed project-scoped GETs and encode an opaque cursor once", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const api = createLiveApi(async (url, init) => {
    calls.push({ url: String(url), init: init ?? {} });
    if (String(url).includes("/pipelines/")) return Response.json(deliveryPipelineFixture);
    if (String(url).includes("/tasks?"))
      return Response.json({ items: [taskSummaryFixture], next_cursor: "opaque+/=" });
    return Response.json(taskDetailFixture);
  });
  const signal = new AbortController().signal;
  const first = await api.tasks(ids.project, null, "synthetic-token", signal);
  await api.tasks(ids.project, first.next_cursor ?? null, "synthetic-token", signal);
  assert.equal(
    (await api.task(ids.project, ids.task, "synthetic-token", signal)).description,
    taskDetailFixture.description,
  );
  assert.equal(
    (await api.pipeline(ids.project, ids.version1, "synthetic-token", signal)).id,
    ids.version1,
  );
  assert.deepEqual(
    calls.map((call) => call.url),
    [
      `/api/projects/${ids.project}/tasks?limit=20`,
      `/api/projects/${ids.project}/tasks?limit=20&cursor=opaque%2B%2F%3D`,
      `/api/projects/${ids.project}/tasks/${ids.task}`,
      `/api/projects/${ids.project}/pipelines/${ids.version1}`,
    ],
  );
  for (const { init } of calls) {
    assert.equal(init.method, "GET");
    assert.equal(init.credentials, "omit");
    assert.equal(init.redirect, "error");
    assert.equal(init.cache, "no-store");
    assert.equal(new Headers(init.headers).get("Authorization"), "Bearer synthetic-token");
    assert.equal(init.signal, signal);
  }
});

test("Task read errors recognize only the agreed status/code pairs without exposing error text", async () => {
  for (const [status, code, kind] of [
    [502, "response_too_large", "response_too_large"],
    [409, "cursor_invalid", "cursor_invalid"],
    [500, "response_too_large", "http"],
    [502, "cursor_invalid", "http"],
    [409, "other_future_code", "http"],
    [401, "cursor_invalid", "unauthorized"],
  ] as const) {
    const api = createLiveApi(async () =>
      Response.json({ error: "secret diagnostic payload", code }, { status }),
    );
    await assert.rejects(
      api.tasks(ids.project, null, "token", new AbortController().signal),
      (error: unknown) => {
        assert.equal((error as { kind: string }).kind, kind);
        assert.equal(describeApiError(error, "Task").includes("secret"), false);
        return true;
      },
    );
  }
});

test("invalid identifiers, wrong scoped IDs, malformed DTOs and oversized pages fail closed", async () => {
  let calls = 0;
  const invalid = createLiveApi(async () => {
    calls += 1;
    return Response.json({});
  });
  const signal = new AbortController().signal;
  await assert.rejects(invalid.tasks("../private", null, "token", signal), { kind: "invalid_id" });
  await assert.rejects(invalid.task(ids.project, "../private", "token", signal), {
    kind: "invalid_id",
  });
  await assert.rejects(invalid.pipeline("bad", ids.version1, "token", signal), {
    kind: "invalid_id",
  });
  assert.equal(calls, 0);
  await assert.rejects(invalid.tasks(ids.project, null, "token", signal), {
    kind: "invalid_response",
  });
  const wrongTask = createLiveApi(async () =>
    Response.json({ ...taskDetailFixture, id: ids.actor }),
  );
  await assert.rejects(wrongTask.task(ids.project, ids.task, "token", signal), {
    kind: "invalid_response",
  });
  const wrongVersion = createLiveApi(async () => Response.json(deliveryPipelineFixture));
  await assert.rejects(wrongVersion.pipeline(ids.project, ids.version2, "token", signal), {
    kind: "invalid_response",
  });
  const tooMany = createLiveApi(async () =>
    Response.json({ items: Array.from({ length: 21 }, () => taskSummaryFixture) }),
  );
  await assert.rejects(tooMany.tasks(ids.project, null, "token", signal), {
    kind: "invalid_response",
  });
});
