import assert from "node:assert/strict";
import { test } from "node:test";
import { AmendEmployeeRequestSchema, amendEmployeeAttempt } from "../contracts/amend-employee.ts";
import { sendAmendEmployee } from "./amend-employee-api.ts";
import { LiveCommandError } from "./command-error.ts";

const request = AmendEmployeeRequestSchema.parse({
  project_id: "01988000-0000-7000-8000-000000000001",
  expected_revision: 3,
  payload: {
    employee_id: "01988000-0000-7000-8000-000000000002",
    expected_employee_revision: 2,
    patch: { name: "New name" },
  },
});
const receipt = {
  command_id: "01988000-0000-7000-8000-000000000003",
  status: "applied",
  project_revision: 4,
  event_ids: ["01988000-0000-7000-8000-000000000004"],
  resource: { kind: "employee", id: request.payload.employee_id },
};

test("Employee amendment sends exact bytes and requires matching receipt", async () => {
  const attempt = amendEmployeeAttempt(request, "11111111-1111-4111-8111-111111111111");
  const seen: { path?: string; body?: string; key?: string | null } = {};
  const fetcher = (async (path: string, init: RequestInit) => {
    seen.path = path;
    seen.body = init.body as string;
    seen.key = new Headers(init.headers).get("Idempotency-Key");
    return Response.json(receipt);
  }) as typeof fetch;
  assert.deepEqual(
    await sendAmendEmployee(
      fetcher,
      attempt,
      "a".repeat(64),
      new AbortController().signal,
      () => new Error("unauthorized"),
    ),
    receipt,
  );
  assert.deepEqual(seen, {
    path: "/api/commands/amend_employee",
    body: attempt.body,
    key: attempt.key,
  });
  for (const invalid of [
    { ...receipt, project_revision: 5 },
    { ...receipt, resource: { kind: "employee", id: request.project_id } },
  ]) {
    await assert.rejects(
      sendAmendEmployee(
        (async () => Response.json(invalid)) as typeof fetch,
        attempt,
        "a".repeat(64),
        new AbortController().signal,
        () => new Error("unauthorized"),
      ),
      (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
    );
  }
});
