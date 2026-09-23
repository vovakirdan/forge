import assert from "node:assert/strict";
import { test } from "node:test";
import { inboxAttempt } from "../contracts/communication-command.ts";
import { sendInboxCommand } from "./inbox-command-api.ts";
import { LiveCommandError } from "./command-error.ts";
const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const thread = "01988000-0000-7000-8000-000000000003";
const event = "01988000-0000-7000-8000-000000000004";
const key = "550e8400-e29b-41d4-a716-446655440000";

test("open thread uses one exact command and validates durable receipt", async () => {
  const attempt = inboxAttempt(
    "open_employee_thread",
    {
      project_id: project,
      expected_revision: 3,
      payload: { employee_id: employee, task_id: null },
    },
    key,
  );
  const seen: { path: string; body: string; key: string | null }[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    seen.push({
      path: String(input),
      body: String(init?.body),
      key: new Headers(init?.headers).get("Idempotency-Key"),
    });
    return Response.json({
      command_id: event,
      status: "replayed",
      project_revision: 4,
      event_ids: [event],
      resource: { kind: "employee_thread", id: thread },
    });
  };
  const result = await sendInboxCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => Error("unauthorized"),
  );
  assert.equal(result.status, "replayed");
  assert.deepEqual(seen, [{ path: "/api/commands/open_employee_thread", body: attempt.body, key }]);
});

test("malformed receipt never proves send and unknown outcome retains replay attempt", async () => {
  const attempt = inboxAttempt(
    "send_employee_message",
    {
      project_id: project,
      expected_revision: 3,
      payload: {
        thread_id: thread,
        expected_thread_revision: 1,
        target: { kind: "inbox" },
        kind: "question",
        requirement: "answered",
        body: "hello",
        reply_to: null,
      },
    },
    key,
  );
  const fetcher: typeof fetch = async () =>
    Response.json({
      command_id: event,
      status: "applied",
      project_revision: 4,
      event_ids: [event],
      resource: { kind: "task", id: thread },
    });
  await assert.rejects(
    sendInboxCommand(fetcher, attempt, "token", new AbortController().signal, () =>
      Error("unauthorized"),
    ),
    (error) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
  assert.equal(JSON.parse(attempt.body).payload.body, "hello");
});
