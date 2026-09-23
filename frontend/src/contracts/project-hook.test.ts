import { test } from "node:test";
import assert from "node:assert/strict";
import { configureProjectHookAttempt, ProjectHookInvocationListSchema } from "./project-hook.ts";

const project = "01988000-0000-7000-8000-000000000001";
const input = {
  name: "lint",
  image: `runner@sha256:${"a".repeat(64)}`,
  command: ["lint"],
  workdir: ".",
  limits: {
    cpu_millis: 2000,
    memory_bytes: 1073741824,
    pids: 64,
    wall_seconds: 300,
    stop_grace_seconds: 10,
  },
  max_output_bytes: 1048576,
  applicable_task_kinds: [],
  required: false,
};
test("hook attempt freezes an exact version request and rejects credential material", () => {
  const attempt = configureProjectHookAttempt(project, 3, input);
  assert.equal(JSON.parse(attempt.body).payload.required, false);
  assert.throws(() => configureProjectHookAttempt(project, 3, { ...input, secret_id: project }));
  assert.throws(() =>
    configureProjectHookAttempt(project, 3, { ...input, image: "runner:latest" }),
  );
});

test("hook result projection accepts only mapped canonical completion and no private payload", () => {
  const row = {
    id: project,
    task_id: project,
    pipeline_version_id: project,
    stage_id: "check",
    stage_visit: 1,
    hook_version_id: project,
    candidate_proposal_id: null,
    run_id: null,
    state: "completed",
    verdict: "skipped",
    mapped_outcome: "skipped",
    artifact_id: project,
    created_at: "2026-09-23T00:00:00Z",
    updated_at: "2026-09-23T00:00:01Z",
  };
  assert.equal(
    ProjectHookInvocationListSchema.parse({ items: [row] }).items[0]?.verdict,
    "skipped",
  );
  assert.throws(() => ProjectHookInvocationListSchema.parse({ items: [{ ...row, result: {} }] }));
  assert.throws(() => ProjectHookInvocationListSchema.parse({ items: [{ ...row, spec: {} }] }));
  assert.throws(() =>
    ProjectHookInvocationListSchema.parse({
      items: [{ ...row, state: "held", mapped_outcome: "passed" }],
    }),
  );
});
