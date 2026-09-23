import assert from "node:assert/strict";
import { test } from "node:test";
import { inboxAttempt } from "./communication-command.ts";
import { MessageDeliveryListSchema } from "./communication.ts";
const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const thread = "01988000-0000-7000-8000-000000000003";
const message = "01988000-0000-7000-8000-000000000004";
const key = "550e8400-e29b-41d4-a716-446655440000";

test("Inbox attempts freeze revision, body and replay key without delivery claims", () => {
  const payload = {
    thread_id: thread,
    expected_thread_revision: 2,
    target: { kind: "inbox" },
    kind: "question",
    requirement: "answered",
    body: "Could you inspect this?",
    reply_to: null,
  };
  const attempt = inboxAttempt(
    "send_employee_message",
    { project_id: project, expected_revision: 7, payload },
    key,
  );
  assert.equal(attempt.key, key);
  assert.equal(JSON.parse(attempt.body).payload.body, payload.body);
  assert.equal(attempt.expectedRevision, 7);
  assert.ok(Object.isFrozen(attempt));
  assert.equal(
    attempt.body,
    inboxAttempt(
      "send_employee_message",
      { project_id: project, expected_revision: 7, payload },
      key,
    ).body,
  );
  assert.throws(() =>
    inboxAttempt(
      "send_employee_message",
      {
        project_id: project,
        expected_revision: 7,
        payload: { ...payload, kind: "reply", requirement: "answered" },
      },
      key,
    ),
  );
});

test("delivery projection keeps persisted receipt and waiver evidence distinct", () => {
  const page = MessageDeliveryListSchema.parse({
    items: [
      {
        message_id: message,
        sequence: 1,
        assignment: { state: "held", attempt_number: 1, run_id: null, retry_ready: false },
        runtime_accepted_at: "2026-09-23T10:00:00Z",
        acknowledged_at: null,
        answered_at: null,
        answered_reply_id: null,
        waiver: {
          message_id: message,
          project_id: project,
          actor: { kind: "human", id: employee },
          reason: "obsolete",
          created_at: "2026-09-23T11:00:00Z",
        },
      },
    ],
  });
  assert.equal(page.items[0]?.assignment?.state, "held");
  assert.equal(page.items[0]?.acknowledged_at, null);
  assert.equal(page.items[0]?.waiver?.reason, "obsolete");
  assert.equal(
    MessageDeliveryListSchema.safeParse({
      items: [{ ...page.items[0], answered_at: "2026-09-23T12:00:00Z" }],
    }).success,
    false,
  );
});

test("Communication retry freezes the held Run, reason, revision and replay key", () => {
  const attempt = inboxAttempt(
    "retry_communication",
    {
      project_id: project,
      expected_revision: 9,
      payload: { run_id: thread, reason: "Lease retired; environment released" },
    },
    key,
  );
  assert.equal(attempt.resourceId, thread);
  assert.equal(attempt.expectedRevision, 9);
  assert.equal(JSON.parse(attempt.body).payload.reason, "Lease retired; environment released");
  assert.equal(
    inboxAttempt(
      "retry_communication",
      {
        project_id: project,
        expected_revision: 9,
        payload: { run_id: thread, reason: "Lease retired; environment released" },
      },
      key,
    ).body,
    attempt.body,
  );
  assert.throws(() =>
    inboxAttempt(
      "retry_communication",
      {
        project_id: project,
        expected_revision: 9,
        payload: { run_id: thread, reason: " " },
      },
      key,
    ),
  );
});
