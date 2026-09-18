import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient } from "@tanstack/react-query";
import { discardRead, invalidateProjectDependencies, readKeys } from "./read-cache.ts";

test("dependency keys isolate each Task, direction, page, Project and session", () => {
  const key = readKeys.dependencies(1, "project", "task", "blocked_by", null);
  assert.deepEqual(key, ["live", 1, "project", "dependencies", "task", "blocked_by", null]);
  for (const other of [
    readKeys.dependencies(2, "project", "task", "blocked_by", null),
    readKeys.dependencies(1, "other", "task", "blocked_by", null),
    readKeys.dependencies(1, "project", "other", "blocked_by", null),
    readKeys.dependencies(1, "project", "task", "blocks", null),
    readKeys.dependencies(1, "project", "task", "blocked_by", "cursor"),
  ])
    assert.notDeepEqual(key, other);
});
test("accepted cancellation invalidates both directions for the Project, not other scopes", async () => {
  const client = new QueryClient();
  const affected = [
    readKeys.dependencies(1, "project", "a", "blocks", null),
    readKeys.dependencies(1, "project", "b", "blocked_by", "next"),
  ];
  const unchanged = [
    readKeys.dependencies(1, "other", "a", "blocks", null),
    readKeys.dependencies(2, "project", "a", "blocks", null),
    readKeys.task(1, "project", "a"),
  ];
  for (const key of [...affected, ...unchanged]) client.setQueryData(key, { items: [] });
  await invalidateProjectDependencies(client, 1, "project");
  for (const key of affected) assert.equal(client.getQueryState(key)?.isInvalidated, true);
  for (const key of unchanged) assert.equal(client.getQueryState(key)?.isInvalidated, false);
  for (const key of affected) discardRead(client, key);
  assert.equal(client.getQueryCache().getAll().length, unchanged.length);
  client.clear();
});
