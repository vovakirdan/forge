import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, describeApiError } from "./api.ts";
import { deliveryPipelineFixture, ids } from "../contracts/fixtures.ts";

test("Pipeline version reads use scoped GETs with fixed page size and encoded cursors", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const api = createLiveApi(async (url, init) => {
    calls.push({ url: String(url), init: init ?? {} });
    return Response.json(
      String(url).includes("/pipelines?")
        ? { items: [deliveryPipelineFixture], next_cursor: "opaque+/=" }
        : deliveryPipelineFixture,
    );
  });
  const signal = new AbortController().signal;
  const first = await api.pipelines(ids.project, null, "synthetic-token", signal);
  await api.pipelines(ids.project, first.next_cursor ?? null, "synthetic-token", signal);
  assert.deepEqual(
    await api.pipeline(ids.project, ids.version1, "synthetic-token", signal),
    deliveryPipelineFixture,
  );
  assert.deepEqual(
    calls.map(({ url }) => url),
    [
      `/api/projects/${ids.project}/pipelines?limit=20`,
      `/api/projects/${ids.project}/pipelines?limit=20&cursor=opaque%2B%2F%3D`,
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
    assert.equal(init.body, undefined);
  }
});

test("Pipeline boundary rejects invalid IDs, wrong version, malformed definitions and oversized pages", async () => {
  const signal = new AbortController().signal;
  let calls = 0;
  const invalid = createLiveApi(async () => {
    calls += 1;
    return Response.json({});
  });
  await assert.rejects(invalid.pipelines("../private", null, "token", signal), {
    kind: "invalid_id",
  });
  await assert.rejects(invalid.pipeline(ids.project, "../private", "token", signal), {
    kind: "invalid_id",
  });
  assert.equal(calls, 0);
  for (const value of [
    {},
    { ...deliveryPipelineFixture, id: ids.version2 },
    {
      ...deliveryPipelineFixture,
      stages: [{ ...deliveryPipelineFixture.stages[0], executor_kind: "unknown" }],
    },
  ]) {
    const api = createLiveApi(async () => Response.json(value));
    await assert.rejects(api.pipeline(ids.project, ids.version1, "token", signal), {
      kind: "invalid_response",
    });
  }
  for (const value of [
    { items: [{}] },
    { items: Array.from({ length: 21 }, () => deliveryPipelineFixture) },
  ]) {
    const api = createLiveApi(async () => Response.json(value));
    await assert.rejects(api.pipelines(ids.project, null, "token", signal), {
      kind: "invalid_response",
    });
  }
});

test("Pipeline errors distinguish oversize and cursor without exposing response text", async () => {
  for (const [status, code, kind] of [
    [404, undefined, "not_found"],
    [401, undefined, "unauthorized"],
    [502, "response_too_large", "response_too_large"],
    [409, "cursor_invalid", "cursor_invalid"],
  ] as const) {
    const api = createLiveApi(async () =>
      Response.json({ error: "private definition", code }, { status }),
    );
    await assert.rejects(
      api.pipelines(ids.project, null, "token", new AbortController().signal),
      (error: unknown) => {
        assert.equal((error as { kind: string }).kind, kind);
        assert.equal(describeApiError(error, "Pipeline").includes("private"), false);
        return true;
      },
    );
  }
});
