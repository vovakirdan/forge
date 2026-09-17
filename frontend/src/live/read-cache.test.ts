import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { prepareProjectChange, prepareReadChange, discardRead, readKeys } from "./read-cache.ts";

test("Task read keys scope session, Project, selected Task and exact Pipeline pin without tokens", () => {
  assert.deepEqual(readKeys.tasks(2, "project", "opaque"), [
    "live",
    2,
    "project",
    "tasks",
    "opaque",
  ]);
  assert.deepEqual(readKeys.task(2, "project", "task"), ["live", 2, "project", "task", "task"]);
  assert.deepEqual(readKeys.pipeline(2, "project", "task", "pin"), [
    "live",
    2,
    "project",
    "pipeline",
    "task",
    "pin",
  ]);
  assert.notDeepEqual(readKeys.task(2, "project", "task"), readKeys.task(3, "project", "task"));
});

test("navigation discards inactive large reads immediately instead of accumulating gcTime caches", () => {
  const client = new QueryClient();
  for (let index = 0; index < 100; index += 1) {
    const key = readKeys.task(1, "project", String(index));
    client.setQueryData(key, { artifacts: ["large artifact body"] });
    assert.equal(client.getQueryCache().getAll().length, 1);
    discardRead(client, key);
    assert.equal(client.getQueryCache().getAll().length, 0);
  }
});

test("scope exit aborts pending requests and late results cannot recreate removed cache entries", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const key = readKeys.task(1, "project", "task");
  let aborted = false;
  let finish!: (value: { title: string }) => void;
  const pending = client.fetchQuery({
    queryKey: key,
    queryFn: ({ signal }) => {
      signal.addEventListener("abort", () => {
        aborted = true;
      });
      return new Promise<{ title: string }>((resolve) => {
        finish = resolve;
      });
    },
  });
  const rejected = assert.rejects(pending);
  discardRead(client, key);
  assert.equal(aborted, true);
  finish({ title: "Late result" });
  await rejected;
  assert.equal(client.getQueryCache().getAll().length, 0);
});

test("Project change drops every Project/Task/Pipeline read of the session but preserves health", () => {
  const client = new QueryClient();
  client.setQueryData(["health", 1], { status: "ready" });
  client.setQueryData(["project", 1, "old"], { name: "old" });
  client.setQueryData(readKeys.tasks(1, "old", null), { items: [] });
  client.setQueryData(readKeys.task(1, "old", "task"), {});
  client.setQueryData(readKeys.pipeline(1, "old", "task", "pin"), {});
  prepareProjectChange(client, 1);
  assert.deepEqual(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.queryKey),
    [["health", 1]],
  );
});

test("Project navigation does not recreate an observed old query during an intermediate render", async (context) => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const oldKey = ["project", 1, "previous"];
  let previousRequests = 0;
  client.setQueryData(oldKey, { name: "Previous Project" });
  const previousOptions = {
    queryKey: oldKey,
    staleTime: Infinity,
    queryFn: async () => {
      previousRequests += 1;
      return { name: "Previous Project" };
    },
  };
  const observer = new QueryObserver(client, previousOptions);
  const unsubscribe = observer.subscribe(() => {});
  context.after(() => {
    unsubscribe();
    client.clear();
  });

  prepareProjectChange(client, 1);
  // A useActionState pending render may still have the old Project ID.
  observer.getOptimisticResult(client.defaultQueryOptions(previousOptions));
  observer.setOptions(previousOptions);
  await Promise.resolve();
  assert.equal(previousRequests, 0, "Navigation must not launch another previous-Project read");

  const selectedKey = ["project", 1, "selected"];
  observer.setOptions({
    ...previousOptions,
    queryKey: selectedKey,
    queryFn: async () => ({ name: "Selected Project" }),
  });
  // Committed scope cleanup owns removal after the observer changes keys.
  discardRead(client, oldKey);
  assert.equal(client.getQueryState(oldKey), undefined);
  assert.deepEqual(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.queryKey),
    [selectedKey],
  );
});

test("page navigation cancels an observed request without recreating it before key commit", async (context) => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const firstKey = readKeys.tasks(1, "project", null);
  let requests = 0;
  let aborted = false;
  let finish!: (value: { items: string[] }) => void;
  const firstOptions = {
    queryKey: firstKey,
    queryFn: ({ signal }: { signal: AbortSignal }) => {
      requests += 1;
      signal.addEventListener("abort", () => {
        aborted = true;
      });
      return new Promise<{ items: string[] }>((resolve) => {
        finish = resolve;
      });
    },
  };
  const observer = new QueryObserver(client, firstOptions);
  const unsubscribe = observer.subscribe(() => {});
  context.after(() => {
    unsubscribe();
    client.clear();
  });
  prepareReadChange(client, firstKey);
  assert.equal(aborted, true);
  assert.ok(client.getQueryState(firstKey), "Observed query survives only until committed cleanup");
  observer.getOptimisticResult(client.defaultQueryOptions(firstOptions));
  observer.setOptions(firstOptions);
  assert.equal(requests, 1, "Intermediate render must not restart the cancelled old page");
  const selectedKey = readKeys.tasks(1, "project", "next");
  observer.setOptions({
    ...firstOptions,
    queryKey: selectedKey,
    queryFn: async () => ({ items: ["selected"] }),
  });
  discardRead(client, firstKey);
  finish({ items: ["stale previous"] });
  await Promise.resolve();
  assert.equal(client.getQueryState(firstKey), undefined);
  assert.deepEqual(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.queryKey),
    [selectedKey],
  );
});
