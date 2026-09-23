import assert from "node:assert/strict";
import { test } from "node:test";
import { systemJobAttempt } from "../contracts/system-job-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendSystemJobCommand } from "./system-job-command-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const job = "01988000-0000-7000-8000-000000000003";
const key = "01988000-0000-4000-8000-000000000004";

test("named System Job actions freeze payload and reject forged operation", () => {
  const attempt = systemJobAttempt(
    "skip_employee_onboarding",
    {
      project_id: project,
      expected_revision: 3,
      payload: { employee_id: employee, reason: "Owner approved" },
    },
    key,
  );
  assert.equal(attempt.resourceKind, "employee");
  assert.equal(attempt.expectedResourceId, employee);
  assert.throws(() =>
    systemJobAttempt(
      "skip_employee_onboarding",
      {
        project_id: project,
        expected_revision: 3,
        payload: { employee_id: employee, reason: " ", operation: "retry" },
      },
      key,
    ),
  );
});

test("System Job retry uses exact bytes/key and checks resource identity", async () => {
  const attempt = systemJobAttempt(
    "retry_system_job",
    { project_id: project, expected_revision: 3, payload: { job_id: job } },
    key,
  );
  const receipt = {
    command_id: job,
    status: "applied",
    project_revision: 4,
    event_ids: [employee],
    resource: { kind: "system_job", id: job },
  };
  const calls: { url: string; body: BodyInit | null | undefined; key: string | null }[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    calls.push({
      url: String(input),
      body: init?.body,
      key: new Headers(init?.headers).get("Idempotency-Key"),
    });
    return Response.json(receipt);
  };
  await sendSystemJobCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  await sendSystemJobCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  assert.deepEqual(calls, [
    { url: "/api/commands/retry_system_job", body: attempt.body, key },
    { url: "/api/commands/retry_system_job", body: attempt.body, key },
  ]);
  await assert.rejects(
    sendSystemJobCommand(
      async () => Response.json({ ...receipt, resource: { kind: "system_job", id: employee } }),
      attempt,
      "token",
      new AbortController().signal,
      () => new Error("unauthorized"),
    ),
    (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});

test("configure requires a typed no-surface binding", () => {
  assert.throws(() =>
    systemJobAttempt(
      "configure_system_jobs",
      {
        project_id: project,
        expected_revision: 3,
        payload: { expected_settings_revision: 0, policy: {}, binding: {} },
      },
      key,
    ),
  );
});
