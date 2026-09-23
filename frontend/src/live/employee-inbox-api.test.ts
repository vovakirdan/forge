import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const thread = "01988000-0000-7000-8000-000000000003";
const message = "01988000-0000-7000-8000-000000000004";
const time = "2026-09-23T12:00:00Z";
const threadView = {
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
const messageView = {
  id: message,
  thread_id: thread,
  project_id: project,
  employee_id: employee,
  sequence: 1,
  sender: { kind: "human", id: employee },
  target: { kind: "inbox" },
  kind: "question",
  requirement: "answered",
  body: "saved text",
  reply_to: null,
  created_at: time,
};

test("Inbox reads use exact Project, Employee, Thread and cursor paths", async () => {
  const seen: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    const path = String(input);
    seen.push(path);
    return Response.json(
      path.includes("/messages?") ? { items: [messageView] } : { items: [threadView] },
    );
  };
  const api = createLiveApi(fetcher);
  await api.employeeThreads(project, employee, thread, "token", new AbortController().signal);
  await api.employeeMessages(project, employee, thread, "0", "token", new AbortController().signal);
  assert.deepEqual(seen, [
    `/api/projects/${project}/employees/${employee}/threads?limit=20&after=${thread}`,
    `/api/projects/${project}/threads/${thread}/messages?limit=20&after=0`,
  ]);
});

test("Inbox reads refuse foreign scope and invalid cursor before rendering", async () => {
  const foreignThreads: typeof fetch = async () =>
    Response.json({ items: [{ ...threadView, employee_id: project }] });
  await assert.rejects(
    createLiveApi(foreignThreads).employeeThreads(
      project,
      employee,
      null,
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
  const foreignMessages: typeof fetch = async () =>
    Response.json({ items: [{ ...messageView, thread_id: project }] });
  await assert.rejects(
    createLiveApi(foreignMessages).employeeMessages(
      project,
      employee,
      thread,
      null,
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
  await assert.rejects(
    createLiveApi(foreignMessages).employeeMessages(
      project,
      employee,
      thread,
      "-1",
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "cursor_invalid",
  );
});
