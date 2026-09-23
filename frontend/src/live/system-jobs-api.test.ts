import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const job = "01988000-0000-7000-8000-000000000003";
const status = {
  policy: null,
  settings_revision: null,
  jobs: [
    {
      id: job,
      project_id: project,
      kind: "onboarding",
      source_task_id: null,
      target_employee_id: employee,
      generation: 1,
      covered_sequence: 0,
      state: "pending",
      reason_code: null,
    },
  ],
  usage: {
    attempts_last_24h: 0,
    running: 0,
    usage_status: "see_per_run_accounting_unknown_is_not_zero",
  },
  onboarding: [
    { employee_id: employee, state: "pending", revision: 1, job_id: job, receipt: null },
  ],
};

test("SystemJob and onboarding reads use only exact scoped routes", async () => {
  const seen: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    const path = String(input);
    seen.push(path);
    return Response.json(path.endsWith("/system-jobs") ? status : status.onboarding[0]);
  };
  const api = createLiveApi(fetcher);
  assert.equal(
    (await api.systemJobs(project, "token", new AbortController().signal)).jobs[0]?.state,
    "pending",
  );
  assert.equal(
    (await api.employeeOnboarding(project, employee, "token", new AbortController().signal)).state,
    "pending",
  );
  assert.deepEqual(seen, [
    `/api/projects/${project}/system-jobs`,
    `/api/projects/${project}/employees/${employee}/onboarding`,
  ]);
});

test("SystemJob read rejects foreign Project job and duplicate onboarding identities", async () => {
  for (const body of [
    { ...status, jobs: [{ ...status.jobs[0], project_id: employee }] },
    { ...status, onboarding: [status.onboarding[0], status.onboarding[0]] },
  ]) {
    const fetcher: typeof fetch = async () => Response.json(body);
    await assert.rejects(
      createLiveApi(fetcher).systemJobs(project, "token", new AbortController().signal),
      (error) => error instanceof LiveApiError && error.kind === "invalid_response",
    );
  }
});

test("SystemJob attempts use scoped cursor route and reject foreign or raw result data", async () => {
  const attempt = {
    id: "01988000-0000-7000-8000-000000000004",
    job_id: job,
    generation: 2,
    run_id: "01988000-0000-7000-8000-000000000005",
    state: "completed",
    result_accepted: true,
    result_message_id: "01988000-0000-7000-8000-000000000006",
    created_at: "2026-09-23T08:00:00Z",
    completed_at: "2026-09-23T08:01:00Z",
    materialized_entries: [{ id: "01988000-0000-7000-8000-000000000007", revision: 1 }],
  };
  const seen: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    seen.push(String(input));
    return Response.json({ items: [attempt] });
  };
  const api = createLiveApi(fetcher);
  const page = await api.systemJobAttempts(
    project,
    job,
    attempt.id,
    "token",
    new AbortController().signal,
  );
  assert.equal(page.items[0]?.run_id, attempt.run_id);
  assert.deepEqual(seen, [
    `/api/projects/${project}/system-jobs/${job}/attempts?limit=20&cursor=${attempt.id}`,
  ]);
  for (const bad of [
    { ...attempt, job_id: employee },
    { ...attempt, result: { markdown: "raw" } },
  ]) {
    await assert.rejects(
      createLiveApi(async () => Response.json({ items: [bad] })).systemJobAttempts(
        project,
        job,
        null,
        "token",
        new AbortController().signal,
      ),
      (error) => error instanceof LiveApiError && error.kind === "invalid_response",
    );
  }
});
