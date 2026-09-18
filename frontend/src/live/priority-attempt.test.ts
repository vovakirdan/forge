import assert from "node:assert/strict";
import { test } from "node:test";
import { ids, taskDetailFixture } from "../contracts/fixtures.ts";
import { TaskDetailViewSchema } from "../contracts/task.ts";
import { PrioritySchemeViewSchema } from "../contracts/priority-scheme.ts";
import { createPriorityAttempt, activePriority } from "./priority-attempt.ts";

const baseline = {
  task: TaskDetailViewSchema.parse({ ...taskDetailFixture, priority: "normal" }),
  scheme: PrioritySchemeViewSchema.parse({
    project_id: ids.project,
    project_revision: 9,
    default_level_id: "normal",
    levels: [
      { id: "normal", display_name: "Normal", rank: -1, retired: false },
      { id: "high", display_name: "High", rank: -1, retired: false },
      { id: "old", display_name: "Old", rank: 10, retired: true },
    ],
  }),
};
const key = "10000000-0000-4000-8000-000000000001";
test("priority attempt uses the scheme revision and freezes exact replay bytes", () => {
  for (const lifecycle of ["draft", "ready", "in_progress", "waiting"] as const) {
    const attempt = createPriorityAttempt(
      { ...baseline, task: { ...baseline.task, lifecycle } },
      "high",
      key,
    );
    assert.deepEqual(JSON.parse(attempt.body), {
      project_id: ids.project,
      expected_revision: 9,
      payload: { task_id: ids.task, expected_task_revision: 7, priority: "high" },
    });
    assert.equal(Object.isFrozen(attempt), true);
    assert.equal(attempt.key, key);
  }
});
test("unchanged, unknown, retired and terminal priority mutations are blocked", () => {
  for (const choice of ["normal", "missing", "old"])
    assert.throws(() => createPriorityAttempt(baseline, choice, key));
  for (const lifecycle of ["done", "cancelled"] as const)
    assert.throws(() =>
      createPriorityAttempt({ ...baseline, task: { ...baseline.task, lifecycle } }, "high", key),
    );
  assert.equal(activePriority(baseline.scheme, "old"), false);
  assert.throws(() => createPriorityAttempt(baseline, "high", "bad"));
  assert.throws(() =>
    createPriorityAttempt(
      { ...baseline, scheme: { ...baseline.scheme, project_revision: Number.MAX_SAFE_INTEGER } },
      "high",
      key,
    ),
  );
});
test("an unknown current level does not prevent explicit selection of an active level", () => {
  assert.doesNotThrow(() =>
    createPriorityAttempt(
      { ...baseline, task: { ...baseline.task, priority: "removed" } },
      "high",
      key,
    ),
  );
});
