import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, describeApiError } from "./api.ts";
import { ids } from "../contracts/fixtures.ts";
import {
  runIds,
  runViewFixture,
  populatedRunDiagnosticsFixture,
} from "../contracts/run-fixtures.ts";

const runDetail = { ...runViewFixture, diagnostics: populatedRunDiagnosticsFixture };

test("Run reads use project-scoped GETs, fixed page size and encoded opaque cursors", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const api = createLiveApi(async (url, init) => {
    calls.push({ url: String(url), init: init ?? {} });
    return Response.json(
      String(url).includes("/runs?")
        ? { items: [runViewFixture], next_cursor: "opaque+/=" }
        : runDetail,
    );
  });
  const signal = new AbortController().signal;
  const first = await api.runs(ids.project, null, "synthetic-token", signal);
  await api.runs(ids.project, first.next_cursor ?? null, "synthetic-token", signal);
  assert.deepEqual(await api.run(ids.project, runIds.run, "synthetic-token", signal), runDetail);
  assert.deepEqual(
    calls.map(({ url }) => url),
    [
      `/api/projects/${ids.project}/runs?limit=20`,
      `/api/projects/${ids.project}/runs?limit=20&cursor=opaque%2B%2F%3D`,
      `/api/projects/${ids.project}/runs/${runIds.run}`,
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

test("Run boundary rejects invalid IDs, wrong Run, missing diagnostics and oversized pages", async () => {
  const signal = new AbortController().signal;
  let calls = 0;
  const invalid = createLiveApi(async () => {
    calls += 1;
    return Response.json({});
  });
  await assert.rejects(invalid.runs("../private", null, "token", signal), { kind: "invalid_id" });
  await assert.rejects(invalid.run(ids.project, "../private", "token", signal), {
    kind: "invalid_id",
  });
  await assert.rejects(invalid.run("bad", runIds.run, "token", signal), { kind: "invalid_id" });
  assert.equal(calls, 0);
  await assert.rejects(invalid.runs(ids.project, null, "token", signal), {
    kind: "invalid_response",
  });
  for (const value of [
    runViewFixture,
    { ...runDetail, id: ids.actor },
    { ...runDetail, observed_state: "done" },
  ]) {
    const api = createLiveApi(async () => Response.json(value));
    await assert.rejects(api.run(ids.project, runIds.run, "token", signal), {
      kind: "invalid_response",
    });
  }
  const tooMany = createLiveApi(async () =>
    Response.json({
      items: Array.from({ length: 21 }, () => runViewFixture),
    }),
  );
  await assert.rejects(tooMany.runs(ids.project, null, "token", signal), {
    kind: "invalid_response",
  });
});

test("Run errors are safe and distinguish missing, unauthorized, oversize and cursor failures", async () => {
  for (const [status, code, kind] of [
    [404, undefined, "not_found"],
    [401, undefined, "unauthorized"],
    [502, "response_too_large", "response_too_large"],
    [409, "cursor_invalid", "cursor_invalid"],
    [502, "cursor_invalid", "http"],
    [409, "response_too_large", "http"],
  ] as const) {
    const api = createLiveApi(async () =>
      Response.json({ error: "private Run body", code }, { status }),
    );
    await assert.rejects(
      api.run(ids.project, runIds.run, "token", new AbortController().signal),
      (error: unknown) => {
        assert.equal((error as { kind: string }).kind, kind);
        const description = describeApiError(error, "Run");
        assert.equal(description.includes("private"), false);
        if (kind === "not_found") assert.equal(description, "Run not found.");
        return true;
      },
    );
  }
});
