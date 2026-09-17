import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { discardRead, prepareSectionChange, readKeys } from "./read-cache.ts";

test("Run keys isolate session, Project, page and selected Run", () => {
  assert.deepEqual(readKeys.runs(2, "project", "cursor"), ["live", 2, "project", "runs", "cursor"]);
  assert.deepEqual(readKeys.run(2, "project", "run"), ["live", 2, "project", "run", "run"]);
  assert.notDeepEqual(readKeys.run(2, "project", "run"), readKeys.run(3, "project", "run"));
  assert.notDeepEqual(readKeys.run(2, "project", "run"), readKeys.run(2, "other", "run"));
  assert.notDeepEqual(readKeys.run(2, "project", "run"), readKeys.task(2, "project", "run"));
});

test("section change removes inactive Task and Run reads but retains Project and health", () => {
  const client = new QueryClient();
  client.setQueryData(["project", 1, "project"], { name: "Project remains" });
  client.setQueryData(["health", 1], { status: "ready" });
  for (const key of [
    readKeys.tasks(1, "project", null),
    readKeys.task(1, "project", "task"),
    readKeys.pipeline(1, "project", "task", "pin"),
    readKeys.runs(1, "project", null),
    readKeys.run(1, "project", "run"),
  ])
    client.setQueryData(key, {});
  prepareSectionChange(client, 1, "project");
  assert.deepEqual(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.queryKey),
    [
      ["project", 1, "project"],
      ["health", 1],
    ],
  );
  client.clear();
});

test("tab switch cancels observed Run without old-query resurrection or Project eviction", async (context) => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const projectKey = ["project", 1, "project"];
  client.setQueryData(projectKey, { name: "Project remains" });
  const runKey = readKeys.run(1, "project", "run");
  let requests = 0;
  let aborted = false;
  let finish!: (value: { body: string }) => void;
  const options = {
    queryKey: runKey,
    queryFn: ({ signal }: { signal: AbortSignal }) => {
      requests += 1;
      signal.addEventListener("abort", () => {
        aborted = true;
      });
      return new Promise<{ body: string }>((resolve) => {
        finish = resolve;
      });
    },
  };
  const observer = new QueryObserver(client, options);
  const unsubscribe = observer.subscribe(() => {});
  context.after(() => {
    unsubscribe();
    client.clear();
  });
  prepareSectionChange(client, 1, "project");
  assert.equal(aborted, true);
  observer.getOptimisticResult(client.defaultQueryOptions(options));
  observer.setOptions(options);
  assert.equal(requests, 1, "Old Run must not restart during intermediate tab render");
  unsubscribe();
  discardRead(client, runKey);
  finish({ body: "Late private diagnostics" });
  await Promise.resolve();
  assert.equal(client.getQueryState(runKey), undefined);
  assert.deepEqual(client.getQueryData(projectKey), { name: "Project remains" });
});

test("Run navigation hard-bounds retained detail bodies instead of waiting for garbage collection", () => {
  const client = new QueryClient();
  const pageKey = readKeys.runs(1, "project", null);
  client.setQueryData(pageKey, { items: [] });
  for (let index = 0; index < 100; index += 1) {
    const key = readKeys.run(1, "project", String(index));
    client.setQueryData(key, { diagnostics: { runtime_report: { body: "opaque" } } });
    assert.equal(client.getQueryCache().getAll().length, 2);
    discardRead(client, key);
    assert.equal(client.getQueryCache().getAll().length, 1);
  }
  client.clear();
});
