import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { EmployeeMessageSchema, MessageDeliverySchema } from "../../src/contracts/communication.ts";
import { EmployeeMessageCard } from "../../src/live/EmployeeInbox.tsx";

test("saved Inbox message renders as plain text without a delivery claim", () => {
  const message = EmployeeMessageSchema.parse({
    id: "01988000-0000-7000-8000-000000000004",
    thread_id: "01988000-0000-7000-8000-000000000003",
    project_id: "01988000-0000-7000-8000-000000000001",
    employee_id: "01988000-0000-7000-8000-000000000002",
    sequence: 1,
    sender: { kind: "human", id: "01988000-0000-7000-8000-000000000002" },
    target: { kind: "inbox" },
    kind: "question",
    requirement: "answered",
    body: '<script>alert("x")</script>',
    reply_to: null,
    created_at: "2026-09-23T12:00:00Z",
  });
  const html = renderToStaticMarkup(<EmployeeMessageCard message={message} />);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>/);
  assert.match(html, /Requested handling: answered/);
  assert.doesNotMatch(html, /Delivered|Acknowledged|Answered by/i);
});

test("delivery evidence separates queued assignment, runtime receipt and waiver", () => {
  const message = EmployeeMessageSchema.parse({
    id: "01988000-0000-7000-8000-000000000004",
    thread_id: "01988000-0000-7000-8000-000000000003",
    project_id: "01988000-0000-7000-8000-000000000001",
    employee_id: "01988000-0000-7000-8000-000000000002",
    sequence: 1,
    sender: { kind: "human", id: "01988000-0000-7000-8000-000000000002" },
    target: { kind: "inbox" },
    kind: "question",
    requirement: "answered",
    body: "Can you answer?",
    reply_to: null,
    created_at: "2026-09-23T12:00:00Z",
  });
  const delivery = MessageDeliverySchema.parse({
    message_id: message.id,
    sequence: 1,
    assignment: { state: "held", attempt_number: 1, run_id: null, retry_ready: false },
    runtime_accepted_at: "2026-09-23T12:01:00Z",
    acknowledged_at: null,
    answered_at: null,
    answered_reply_id: null,
    waiver: {
      message_id: message.id,
      project_id: message.project_id,
      actor: { kind: "human", id: message.employee_id },
      reason: "<img src=x onerror=alert(1)>",
      created_at: "2026-09-23T12:02:00Z",
    },
  });
  const html = renderToStaticMarkup(<EmployeeMessageCard message={message} delivery={delivery} />);
  assert.match(html, /Assignment held/);
  assert.match(html, /Runtime accepted/);
  assert.match(html, /No acknowledgement receipt/);
  assert.match(html, /No answer receipt/);
  assert.match(html, /Waived by management/);
  assert.match(html, /&lt;img/);
  assert.doesNotMatch(html, /<img/);
});
