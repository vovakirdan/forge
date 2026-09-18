import assert from "node:assert/strict";
import { test } from "node:test";
import { projectFixture, taskDetailFixture } from "../contracts/fixtures.ts";
import { ProjectViewSchema } from "../contracts/project.ts";
import { TaskDetailViewSchema } from "../contracts/task.ts";
import { createCancellationAttempt } from "./cancellation-attempt.ts";

const baseline = {
  project: ProjectViewSchema.parse(projectFixture),
  task: TaskDetailViewSchema.parse(taskDetailFixture),
  catalog: {
    project_id: projectFixture.id,
    project_revision: projectFixture.revision,
    reasons: [
      { id: "unspecified", display_name: "Unspecified", retired: false },
      { id: "old", display_name: "Old", retired: true },
    ],
  },
};
const key = "10000000-0000-4000-8000-000000000001";
test("cancellation freezes current revisions and preserves note text without defaulting reason", () => {
  const attempt = createCancellationAttempt(baseline, "unspecified", "  Reason\n", key);
  assert.equal(Object.isFrozen(attempt), true);
  assert.deepEqual(JSON.parse(attempt.body), {
    project_id: baseline.project.id,
    expected_revision: baseline.project.revision,
    payload: {
      task_id: baseline.task.id,
      expected_task_revision: baseline.task.revision,
      cancellation_reason_key: "unspecified",
      note: "  Reason\n",
    },
  });
  assert.equal(
    "note" in
      JSON.parse(createCancellationAttempt(baseline, "unspecified", "\u0085\n", key).body).payload,
    false,
  );
  for (const reason of ["", "old", "missing"])
    assert.throws(() => createCancellationAttempt(baseline, reason, "", key));
});
test("cancellation refuses terminal tasks and mismatched catalog revisions and scope", () => {
  for (const lifecycle of ["done", "cancelled"] as const)
    assert.throws(() =>
      createCancellationAttempt(
        { ...baseline, task: { ...baseline.task, lifecycle } },
        "unspecified",
        "",
        key,
      ),
    );
  assert.throws(() =>
    createCancellationAttempt(
      {
        ...baseline,
        catalog: { ...baseline.catalog, project_revision: baseline.project.revision + 1 },
      },
      "unspecified",
      "",
      key,
    ),
  );
  assert.throws(() =>
    createCancellationAttempt(
      { ...baseline, catalog: { ...baseline.catalog, project_id: baseline.task.id } },
      "unspecified",
      "",
      key,
    ),
  );
});
