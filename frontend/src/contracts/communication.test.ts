import assert from "node:assert/strict";
import { test } from "node:test";
import { EmployeeMessageListSchema, EmployeeThreadListSchema } from "./communication.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const thread = "01988000-0000-7000-8000-000000000003";
const message = "01988000-0000-7000-8000-000000000004";
const time = "2026-09-23T12:00:00Z";

export const threadFixture = {
  id: thread,
  project_id: project,
  employee_id: employee,
  task_id: null,
  created_by: { kind: "human", id: employee },
  created_at: time,
  updated_at: time,
  revision: 2,
  last_sequence: 1,
};
export const messageFixture = {
  id: message,
  thread_id: thread,
  project_id: project,
  employee_id: employee,
  sequence: 1,
  sender: { kind: "human", id: employee },
  target: { kind: "inbox" },
  kind: "question",
  requirement: "answered",
  body: "<script>plain text</script>",
  reply_to: null,
  created_at: time,
};

test("Inbox schemas retain taskless threads and durable messages without delivery state", () => {
  const threads = EmployeeThreadListSchema.parse({ items: [threadFixture] });
  const messages = EmployeeMessageListSchema.parse({ items: [messageFixture] });
  assert.equal(threads.items[0]?.task_id, null);
  assert.equal(messages.items[0]?.body, "<script>plain text</script>");
  assert.equal(messages.items[0]?.requirement, "answered");
  assert.equal("delivered" in (messages.items[0] ?? {}), false);
});

test("Inbox schemas reject unsafe sequences and malformed pagination", () => {
  assert.equal(
    EmployeeMessageListSchema.safeParse({ items: [{ ...messageFixture, sequence: 2 ** 53 }] })
      .success,
    false,
  );
  assert.equal(
    EmployeeMessageListSchema.safeParse({ items: [messageFixture], next_cursor: "-1" }).success,
    false,
  );
  assert.equal(
    EmployeeThreadListSchema.safeParse({ items: [threadFixture], next_cursor: null }).success,
    false,
  );
});
