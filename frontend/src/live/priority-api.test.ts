import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi } from "./api.ts";

const projectId = "01a08a41-977c-7d23-a347-22c7afcfb653";
const scheme = {
  project_id: projectId,
  project_revision: 42,
  default_level_id: "own",
  levels: [{ id: "own", display_name: "Owner label", rank: -5, retired: false }],
};

test("priority read uses one fixed project route and the existing safe request boundary", async () => {
  const signal = new AbortController().signal;
  const api = createLiveApi(async (url, init) => {
    assert.equal(url, `/api/projects/${projectId}/priority-scheme`);
    assert.equal(init?.method, "GET");
    assert.equal(init?.body, undefined);
    assert.equal(init?.credentials, "omit");
    assert.equal(init?.redirect, "error");
    assert.equal(init?.cache, "no-store");
    assert.equal(init?.signal, signal);
    assert.equal(new Headers(init?.headers).get("Authorization"), "Bearer synthetic");
    return Response.json(scheme);
  });
  assert.deepEqual(await api.priorityScheme(projectId, "synthetic", signal), scheme);
});

test("priority reads reject invalid IDs before transport and mismatched project responses", async () => {
  let calls = 0;
  const api = createLiveApi(async () => {
    calls += 1;
    return Response.json(scheme);
  });
  const signal = new AbortController().signal;
  await assert.rejects(api.priorityScheme("../secret", "token", signal), { kind: "invalid_id" });
  assert.equal(calls, 0);
  await assert.rejects(
    api.priorityScheme("01a08a41-98c2-7e73-9bdd-df32cb201b61", "token", signal),
    { kind: "invalid_response" },
  );
  assert.deepEqual(await api.priorityScheme(projectId.toUpperCase(), "token", signal), scheme);
});

test("priority schema, transport, size and HTTP failures expose bounded existing error kinds", async () => {
  const signal = new AbortController().signal;
  const cases = [
    { response: Response.json({ ...scheme, levels: [] }), kind: "invalid_response" },
    { response: new Response("private raw data", { status: 401 }), kind: "unauthorized" },
    { response: new Response("private raw data", { status: 404 }), kind: "not_found" },
    { response: new Response("private raw data", { status: 503 }), kind: "http" },
    {
      response: Response.json(
        { error: "private details", code: "response_too_large" },
        { status: 502 },
      ),
      kind: "response_too_large",
    },
  ];
  for (const { response, kind } of cases) {
    await assert.rejects(
      createLiveApi(async () => response).priorityScheme(projectId, "token", signal),
      { kind },
    );
  }
  const api = createLiveApi(async () => {
    throw new Error("private transport");
  });
  await assert.rejects(api.priorityScheme(projectId, "token", signal), { kind: "network" });
  const cancelled = new AbortController();
  cancelled.abort();
  await assert.rejects(api.priorityScheme(projectId, "token", cancelled.signal), {
    name: "AbortError",
  });
});
