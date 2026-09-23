import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";
import { managementAttempt } from "../contracts/management-command.ts";
import { sendManagementCommand } from "./management-command-api.ts";
import { LiveCommandError } from "./command-error.ts";
const project = "01988000-0000-7000-8000-000000000001";
const task = "01988000-0000-7000-8000-000000000002";
const event = "01988000-0000-7000-8000-000000000003";
const key = "550e8400-e29b-41d4-a716-446655440000";
test("management lists use scoped bounded cursor routes", async () => {
  const paths: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    paths.push(String(input));
    return Response.json({ items: [] });
  };
  const api = createLiveApi(fetcher);
  await api.managementPage(
    project,
    "resume-schedules",
    task,
    "token",
    new AbortController().signal,
  );
  await api.managementPage(project, "escalations", null, "token", new AbortController().signal);
  assert.deepEqual(paths, [
    `/api/projects/${project}/resume-schedules?limit=20&cursor=${task}`,
    `/api/projects/${project}/escalations?limit=20`,
  ]);
});
test("management command validates scoped receipt and preserves exact replay body", async () => {
  const attempt = managementAttempt(
    "pause_task",
    {
      project_id: project,
      expected_revision: 7,
      payload: { task_id: task, expected_task_revision: 2, mode: "graceful", reason: "operator" },
    },
    key,
  );
  const seen: string[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    seen.push(String(input));
    assert.equal(init?.body, attempt.body);
    return Response.json({
      command_id: event,
      status: "replayed",
      project_revision: 9,
      event_ids: [event],
      resource: { kind: "task", id: task },
    });
  };
  const receipt = await sendManagementCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => Error("auth"),
  );
  assert.equal(receipt.status, "replayed");
  assert.deepEqual(seen, ["/api/commands/pause_task"]);
  const bad: typeof fetch = async () =>
    Response.json({
      command_id: event,
      status: "applied",
      project_revision: 9,
      event_ids: [event],
      resource: { kind: "project", id: project },
    });
  await assert.rejects(
    sendManagementCommand(bad, attempt, "token", new AbortController().signal, () => Error("auth")),
    (error) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});
test("management list rejects malformed cursor scope before rendering", async () => {
  const fetcher: typeof fetch = async () =>
    Response.json({ items: [{ id: task }], next_cursor: project });
  await assert.rejects(
    createLiveApi(fetcher).managementPage(
      project,
      "resume-schedules",
      null,
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});

test("management facts use scoped cursor shapes and keep taskless recovery Runs", async () => {
  const paths: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    const path = String(input);
    paths.push(path);
    if (path.endsWith("/recovery"))
      return Response.json({ boot_policy: "manual_hold", hold: true });
    if (path.includes("recovery-runs"))
      return Response.json({
        items: [
          {
            run_id: task,
            task_id: null,
            desired_state: "stopped",
            observed_state: "lost",
            created_at: "2026-09-23T10:00:00Z",
            started_at: null,
            liveness_observed_at: null,
            stop_requested_at: null,
            updated_at: "2026-09-23T10:01:00Z",
            lease_active: false,
            reservation_state: "quiescent",
            reservation_released: true,
            accepted_assessment: null,
            assessment_accepted_at: null,
            assessment_command_id: null,
            recovery_queue_entry_id: null,
          },
        ],
      });
    return Response.json({ items: [] });
  };
  const api = createLiveApi(fetcher);
  const signal = new AbortController().signal;
  const routes = await api.managementFactsPage(
    project,
    "resolver-routes",
    "human_route",
    "token",
    signal,
  );
  const constraints = await api.managementFactsPage(
    project,
    "next-run-constraints",
    task,
    "token",
    signal,
  );
  const runs = await api.managementFactsPage(project, "recovery-runs", null, "token", signal);
  const settings = await api.recoverySettings(project, "token", signal);
  assert.equal(routes.kind, "resolver-routes");
  assert.equal(constraints.kind, "next-run-constraints");
  assert.equal(runs.kind, "recovery-runs");
  if (runs.kind !== "recovery-runs") throw Error("wrong kind");
  assert.equal(runs.value.items[0]?.task_id, null);
  assert.equal(settings.hold, true);
  assert.deepEqual(paths, [
    `/api/projects/${project}/resolver-routes?limit=20&cursor=human_route`,
    `/api/projects/${project}/next-run-constraints?limit=20&cursor=${task}`,
    `/api/projects/${project}/recovery-runs?limit=20`,
    `/api/projects/${project}/recovery`,
  ]);
});

test("recovery readiness uses exact Run scope and rejects mismatched or inconsistent facts", async () => {
  const path = `/api/projects/${project}/recovery-runs/${task}/assessment-readiness`;
  const payload = {
    run_id: task,
    assessment: "not_started_confirmed",
    eligible: false,
    reason_code: "physical_state_unresolved",
    project_revision: 7,
    run_revision: 2,
    lease_fencing_token: 3,
    environment_epoch: 1,
    task_id: task,
    task_revision: 4,
  };
  const paths: string[] = [];
  const api = createLiveApi(async (input) => {
    paths.push(String(input));
    return Response.json(payload);
  });
  const value = await api.recoveryAssessmentReadiness(
    project,
    task,
    "token",
    new AbortController().signal,
  );
  assert.equal(value.reason_code, "physical_state_unresolved");
  assert.deepEqual(paths, [path]);
  await assert.rejects(
    createLiveApi(async () =>
      Response.json({ ...payload, eligible: true }),
    ).recoveryAssessmentReadiness(project, task, "token", new AbortController().signal),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});

test("recovery assessment accepts only correlated Run receipt and exact replay", async () => {
  const attempt = managementAttempt(
    "accept_run_recovery_assessment",
    {
      project_id: project,
      expected_revision: 12,
      payload: { run_id: task, assessment: "not_started_confirmed" },
    },
    key,
  );
  const seen: string[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    seen.push(String(input));
    assert.equal(init?.body, attempt.body);
    return Response.json({
      command_id: event,
      status: "replayed",
      project_revision: 14,
      event_ids: [event],
      resource: { kind: "run", id: task },
    });
  };
  const receipt = await sendManagementCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => Error("auth"),
  );
  assert.equal(receipt.status, "replayed");
  assert.deepEqual(seen, ["/api/commands/accept_run_recovery_assessment"]);
  const bad: typeof fetch = async () =>
    Response.json({
      command_id: event,
      status: "applied",
      project_revision: 14,
      event_ids: [event],
      resource: { kind: "task", id: task },
    });
  await assert.rejects(
    sendManagementCommand(bad, attempt, "token", new AbortController().signal, () => Error("auth")),
    (error) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});
