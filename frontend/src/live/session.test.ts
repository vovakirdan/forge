import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient } from "@tanstack/react-query";
import { createLiveApi, LiveApiError } from "./api.ts";
import { createLiveSession, SESSION_STORAGE_KEY } from "./session.ts";

function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}
const saved = { token: "b".repeat(64), expires_at: "2099-01-01T00:00:00Z" };
const successfulApi = () =>
  createLiveApi(async (url) =>
    String(url).endsWith("logout") ? new Response(null, { status: 204 }) : Response.json(saved),
  );

test("session restores only valid unexpired sessionStorage and never exposes token in snapshot", () => {
  const store = storage();
  store.setItem(SESSION_STORAGE_KEY, JSON.stringify(saved));
  const session = createLiveSession(successfulApi(), new QueryClient(), () => store);
  assert.equal(session.getSnapshot().status, "authenticated");
  assert.equal(JSON.stringify(session.getSnapshot()).includes(saved.token), false);
  session.dispose();
  for (const value of [
    "{",
    JSON.stringify({ token: "x", expires_at: "2000-01-01T00:00:00Z" }),
    JSON.stringify({ token: "x" }),
  ]) {
    store.setItem(SESSION_STORAGE_KEY, value);
    const invalid = createLiveSession(successfulApi(), new QueryClient(), () => store);
    assert.equal(invalid.getSnapshot().status, "logged_out");
    assert.equal(store.getItem(SESSION_STORAGE_KEY), null);
    invalid.dispose();
  }
});

test("storage failures do not fall back to in-memory sessions", async () => {
  const queryClient = new QueryClient();
  const unavailable = createLiveSession(successfulApi(), queryClient, () => {
    throw new Error("private storage error");
  });
  assert.equal(unavailable.getSnapshot().status, "storage_unavailable");
  await unavailable.connect("code");
  assert.equal(unavailable.getSnapshot().status, "storage_unavailable");
  unavailable.dispose();
  const store = storage();
  store.setItem = () => {
    throw new Error("private quota error");
  };
  const session = createLiveSession(successfulApi(), queryClient, () => store);
  await session.connect("code");
  assert.equal(session.getSnapshot().status, "storage_unavailable");
  assert.equal(JSON.stringify(session.getSnapshot()).includes("private"), false);
  session.dispose();
});

test("401 clears session and entire query cache; network errors keep authentication", async () => {
  const store = storage();
  const queryClient = new QueryClient();
  const session = createLiveSession(successfulApi(), queryClient, () => store);
  await session.connect("code");
  const generation = session.getSnapshot().generation;
  await assert.rejects(
    session.request(generation, async () => {
      throw new LiveApiError("network");
    }),
    { kind: "network" },
  );
  assert.equal(session.getSnapshot().status, "authenticated");
  queryClient.setQueryData(["project", generation, "id"], { name: "private project" });
  await assert.rejects(
    session.request(generation, async () => {
      throw new LiveApiError("unauthorized");
    }),
    { kind: "unauthorized" },
  );
  assert.equal(session.getSnapshot().status, "logged_out");
  assert.equal(store.getItem(SESSION_STORAGE_KEY), null);
  assert.equal(queryClient.getQueryCache().getAll().length, 0);
  session.dispose();
});

test("logout clears immediately, aborts queries, rejects late responses and reports failed revocation", async () => {
  const store = storage();
  store.setItem(SESSION_STORAGE_KEY, JSON.stringify(saved));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const api = createLiveApi(async () => {
    throw new Error("offline");
  });
  const session = createLiveSession(api, client, () => store);
  const generation = session.getSnapshot().generation;
  let resolve!: (value: string) => void;
  const delayed = session.request(
    generation,
    () =>
      new Promise<string>((done) => {
        resolve = done;
      }),
  );
  let aborted = false;
  const pending = client
    .fetchQuery({
      queryKey: ["pending", generation],
      queryFn: ({ signal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => {
            aborted = true;
            reject(new Error("aborted"));
          });
        }),
    })
    .catch(() => undefined);
  const logout = session.logout();
  assert.equal(session.getSnapshot().status, "logged_out");
  assert.equal(store.getItem(SESSION_STORAGE_KEY), null);
  assert.equal(aborted, true);
  resolve("late private project");
  await assert.rejects(delayed, { name: "AbortError" });
  await Promise.all([logout, pending]);
  assert.match(session.getSnapshot().notice ?? "", /revocation could not be confirmed/);
  assert.equal(client.getQueryCache().getAll().length, 0);
  session.dispose();
});

test("old unauthorized response cannot clear a newly authenticated generation", async () => {
  const session = createLiveSession(successfulApi(), new QueryClient(), storage);
  await session.connect("first");
  let reject!: (error: Error) => void;
  const old = session.request(
    session.getSnapshot().generation,
    () =>
      new Promise((_resolve, fail) => {
        reject = fail;
      }),
  );
  await session.logout();
  await session.connect("second");
  reject(new LiveApiError("unauthorized"));
  await assert.rejects(old, { name: "AbortError" });
  assert.equal(session.getSnapshot().status, "authenticated");
  session.dispose();
});

test("local expiry clears storage and cached data without waiting for another request", (context) => {
  context.mock.timers.enable({
    apis: ["setTimeout", "Date"],
    now: new Date("2098-12-31T23:59:59Z"),
  });
  const store = storage();
  store.setItem(SESSION_STORAGE_KEY, JSON.stringify(saved));
  const client = new QueryClient();
  const session = createLiveSession(successfulApi(), client, () => store);
  client.setQueryData(["project"], { id: "private" });
  context.mock.timers.tick(1001);
  assert.equal(session.getSnapshot().status, "logged_out");
  assert.equal(store.getItem(SESSION_STORAGE_KEY), null);
  assert.equal(client.getQueryCache().getAll().length, 0);
  session.dispose();
});

test("an exchange that finishes after logout cannot restore authentication", async () => {
  let resolve!: (response: Response) => void;
  const api = createLiveApi(
    () =>
      new Promise<Response>((done) => {
        resolve = done;
      }),
  );
  const store = storage();
  const session = createLiveSession(api, new QueryClient(), () => store);
  const connect = session.connect("code");
  assert.equal(session.getSnapshot().status, "authenticating");
  await session.logout();
  resolve(Response.json(saved));
  await connect;
  assert.equal(session.getSnapshot().status, "logged_out");
  assert.equal(store.getItem(SESSION_STORAGE_KEY), null);
  session.dispose();
});

test("expiry is checked on late responses even before a throttled browser timer runs", async (context) => {
  context.mock.timers.enable({ apis: ["Date"], now: new Date("2098-12-31T23:59:59Z") });
  const store = storage();
  store.setItem(SESSION_STORAGE_KEY, JSON.stringify(saved));
  const session = createLiveSession(successfulApi(), new QueryClient(), () => store);
  let resolve!: (value: string) => void;
  const request = session.request(
    session.getSnapshot().generation,
    () =>
      new Promise<string>((done) => {
        resolve = done;
      }),
  );
  context.mock.timers.tick(1001);
  resolve("late data");
  await assert.rejects(request, { name: "AbortError" });
  assert.equal(session.getSnapshot().status, "logged_out");
  session.dispose();
});
