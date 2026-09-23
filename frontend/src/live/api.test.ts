import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const id = "01a08a41-977c-7d23-a347-22c7afcfb653";
const session = { token: "a".repeat(64), expires_at: "2099-01-01T00:00:00Z" };

test("live API uses fixed same-origin routes, headers, no cookies or redirects", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const api = createLiveApi(async (url, init) => {
    calls.push({ url: String(url), init: init ?? {} });
    if (String(url).endsWith("exchange")) return Response.json(session);
    if (String(url).endsWith("logout")) return new Response(null, { status: 204 });
    if (String(url).endsWith("health"))
      return Response.json({ status: "ready", api_version: "v1" });
    return Response.json({
      id,
      name: "<img src=x onerror=alert(1)>",
      revision: 1,
      execution_gate: "open",
    });
  });
  const signal = new AbortController().signal;
  assert.deepEqual(await api.exchange("synthetic-code", signal), session);
  await api.health(session.token, signal);
  assert.equal((await api.project(id, session.token, signal)).name, "<img src=x onerror=alert(1)>");
  await api.logout(session.token, signal);
  assert.deepEqual(
    calls.map((call) => call.url),
    ["/api/auth/exchange", "/api/health", `/api/projects/${id}`, "/api/auth/logout"],
  );
  for (const { url, init } of calls) {
    assert.equal(init.credentials, "omit");
    assert.equal(init.redirect, "error");
    assert.equal(init.cache, "no-store");
    assert.equal(init.signal, signal);
    const headers = new Headers(init.headers);
    assert.equal(
      headers.get("Authorization"),
      url.endsWith("exchange") ? null : `Bearer ${session.token}`,
    );
    assert.equal(url.includes(session.token), false);
  }
  assert.equal(calls[0]?.init.body, JSON.stringify({ code: "synthetic-code" }));
});

test("errors are bounded categories, never server or transport payloads", async () => {
  for (const [status, kind] of [
    [401, "unauthorized"],
    [404, "not_found"],
    [429, "http"],
    [502, "http"],
  ] as const) {
    const api = createLiveApi(async () => new Response("secret raw body", { status }));
    await assert.rejects(
      api.health("secret-token", new AbortController().signal),
      (error: unknown) => {
        assert.ok(error instanceof LiveApiError);
        assert.equal(error.kind, kind);
        assert.equal(String(error).includes("secret"), false);
        return true;
      },
    );
  }
  const api = createLiveApi(async () => {
    throw new Error("secret transport details");
  });
  await assert.rejects(api.health("token", new AbortController().signal), { kind: "network" });
});

test("malformed responses and Project IDs fail closed without echoing values", async () => {
  let requests = 0;
  const api = createLiveApi(async () => {
    requests += 1;
    return Response.json({ token: "secret", expires_at: "never" });
  });
  await assert.rejects(api.exchange("code", new AbortController().signal), {
    kind: "invalid_response",
  });
  await assert.rejects(api.project("../../secret", "token", new AbortController().signal), {
    kind: "invalid_id",
  });
  assert.equal(requests, 1);
  await assert.rejects(api.project(id, "token", new AbortController().signal), {
    kind: "invalid_response",
  });
});

test("Project catalog requests bounded pages and rejects duplicate or oversized results", async () => {
  const calls: string[] = [];
  const project = { id, name: "Safe Project", revision: 1, execution_gate: "stopped" };
  const api = createLiveApi(async (input) => {
    calls.push(String(input));
    return Response.json({ items: [project], next_cursor: id });
  });
  const signal = new AbortController().signal;
  assert.equal((await api.projects(null, "token", signal)).items[0]?.id, id);
  await api.projects("page marker", "token", signal);
  assert.deepEqual(calls, ["/api/projects?limit=20", "/api/projects?limit=20&cursor=page+marker"]);
  for (const items of [[project, project], Array(21).fill(project)]) {
    const invalid = createLiveApi(async () => Response.json({ items }));
    await assert.rejects(invalid.projects(null, "token", signal), { kind: "invalid_response" });
  }
});

test("aborted requests stay cancelled and mismatched Project responses are not accepted", async () => {
  const cancelled = new AbortController();
  cancelled.abort();
  const offline = createLiveApi(async () => {
    throw new Error("transport abort");
  });
  await assert.rejects(offline.health("token", cancelled.signal), { name: "AbortError" });
  const wrong = createLiveApi(async () =>
    Response.json({
      id: "01a08a41-98c2-7e73-9bdd-df32cb201b61",
      name: "Other project",
      revision: 1,
      execution_gate: "open",
    }),
  );
  await assert.rejects(wrong.project(id, "token", new AbortController().signal), {
    kind: "invalid_response",
  });
  for (const response of [
    { status: "ok", api_version: "v1" },
    { status: "ready", api_version: "" },
  ]) {
    const api = createLiveApi(async () => Response.json(response));
    await assert.rejects(api.health("token", new AbortController().signal), {
      kind: "invalid_response",
    });
  }
});
