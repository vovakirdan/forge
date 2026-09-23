import { test } from "node:test";
import assert from "node:assert/strict";
import { resolverAttempt } from "./resolver-command.ts";

const project = "01988000-0000-7000-8000-000000000001";
const task = "01988000-0000-7000-8000-000000000002";
const employee = "01988000-0000-7000-8000-000000000003";
test("resolver route is bounded and rejects duplicate candidates", () => {
  const request = {
    project_id: project,
    expected_revision: 3,
    payload: { route_key: "review", employee_ids: [employee], assignment_timeout_seconds: 300 },
  };
  assert.equal(resolverAttempt("configure_resolver_route", request).routeKey, "review");
  assert.throws(() =>
    resolverAttempt("configure_resolver_route", {
      ...request,
      payload: { ...request.payload, employee_ids: [employee, employee] },
    }),
  );
});
test("escalation requires a Task revision and bounded question", () => {
  const request = {
    project_id: project,
    expected_revision: 3,
    payload: {
      task_id: task,
      expected_task_revision: 2,
      route_key: null,
      category: "clarification",
      question: "Decide",
    },
  };
  assert.equal(resolverAttempt("raise_escalation", request).taskId, task);
  assert.throws(() =>
    resolverAttempt("raise_escalation", {
      ...request,
      payload: { ...request.payload, question: " " },
    }),
  );
});
