import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { discardRead, prepareSectionChange, readKeys } from "./read-cache.ts";

test("Pipeline inspector cache is isolated from Task pins, sessions, Projects and pages", () => {
  assert.deepEqual(readKeys.pipelineVersions(2, "project", "cursor"), [
    "live",
    2,
    "project",
    "pipeline-versions",
    "cursor",
  ]);
  const key = readKeys.pipelineVersion(2, "project", "version");
  assert.notDeepEqual(key, readKeys.pipeline(2, "project", "task", "version"));
  assert.notDeepEqual(key, readKeys.pipelineVersion(3, "project", "version"));
  assert.notDeepEqual(key, readKeys.pipelineVersion(2, "other", "version"));
  assert.notDeepEqual(key, readKeys.pipelineVersion(2, "project", "other"));
});

test("Pipeline detail cancellation does not restart observed reads or resurrect a late definition", async (context) => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const key = readKeys.pipelineVersion(1, "project", "version");
  const projectKey = ["project", 1, "project"];
  client.setQueryData(projectKey, { name: "Retained Project" });
  let requests = 0;
  let aborted = false;
  let finish!: (value: { name: string }) => void;
  const options = {
    queryKey: key,
    queryFn: ({ signal }: { signal: AbortSignal }) => {
      requests += 1;
      signal.addEventListener("abort", () => {
        aborted = true;
      });
      return new Promise<{ name: string }>((resolve) => {
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
  assert.equal(requests, 1);
  unsubscribe();
  discardRead(client, key);
  finish({ name: "Late definition" });
  await Promise.resolve();
  assert.equal(client.getQueryState(key), undefined);
  assert.deepEqual(client.getQueryData(projectKey), { name: "Retained Project" });
});

test("Pipeline navigation releases full definitions immediately, including inactive pages", () => {
  const client = new QueryClient();
  const pageKey = readKeys.pipelineVersions(1, "project", null);
  client.setQueryData(pageKey, { items: [] });
  for (let index = 0; index < 100; index += 1) {
    const key = readKeys.pipelineVersion(1, "project", String(index));
    client.setQueryData(key, { name: "Definition" });
    assert.equal(client.getQueryCache().getAll().length, 2);
    discardRead(client, key);
    assert.equal(client.getQueryCache().getAll().length, 1);
  }
  prepareSectionChange(client, 1, "project");
  assert.equal(client.getQueryCache().getAll().length, 0);
  client.clear();
});
