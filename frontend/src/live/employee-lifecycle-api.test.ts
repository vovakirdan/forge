import assert from "node:assert/strict";
import { test } from "node:test";
import {
  EmployeeLifecycleRequestSchema,
  employeeLifecycleAttempt,
} from "../contracts/employee-lifecycle.ts";
import { sendEmployeeLifecycle } from "./employee-lifecycle-api.ts";
import { LiveCommandError } from "./command-error.ts";

const request = EmployeeLifecycleRequestSchema.parse({
  project_id: "01988000-0000-7000-8000-000000000001",
  expected_revision: 3,
  payload: {
    employee_id: "01988000-0000-7000-8000-000000000002",
    expected_employee_revision: 2,
    reason: "Owner decision",
  },
});
const receipt = {
  command_id: "01988000-0000-7000-8000-000000000003",
  status: "applied",
  project_revision: 4,
  event_ids: ["01988000-0000-7000-8000-000000000004"],
  resource: { kind: "employee", id: request.payload.employee_id },
};

test("Employee state attempt preserves exact bytes and checks receipt", async () => {
  for (const action of ["enable_employee", "disable_employee", "retire_employee"] as const) {
    const attempt = employeeLifecycleAttempt(
      action,
      request,
      "11111111-1111-4111-8111-111111111111",
    );
    const seen: { path?: string; body?: string; key?: string | null } = {};
    const fetcher = (async (path: string, init: RequestInit) => {
      seen.path = path;
      seen.body = init.body as string;
      seen.key = new Headers(init.headers).get("Idempotency-Key");
      return Response.json(receipt);
    }) as typeof fetch;
    assert.deepEqual(
      await sendEmployeeLifecycle(
        fetcher,
        attempt,
        "a".repeat(64),
        new AbortController().signal,
        () => new Error("unauthorized"),
      ),
      receipt,
    );
    assert.deepEqual(seen, {
      path: `/api/commands/${action}`,
      body: attempt.body,
      key: attempt.key,
    });
    for (const wrong of [
      { ...receipt, project_revision: 5 },
      { ...receipt, resource: { kind: "employee", id: request.project_id } },
    ]) {
      await assert.rejects(
        sendEmployeeLifecycle(
          (async () => Response.json(wrong)) as typeof fetch,
          attempt,
          "a".repeat(64),
          new AbortController().signal,
          () => new Error("unauthorized"),
        ),
        (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
      );
    }
  }
});

test("Reason is optional and invalid audit text is refused", () => {
  const noReason = {
    ...request,
    payload: { employee_id: request.payload.employee_id, expected_employee_revision: 2 },
  };
  assert.equal(EmployeeLifecycleRequestSchema.safeParse(noReason).success, true);
  for (const reason of [" ", "a\0b", "x".repeat(10_001)]) {
    assert.equal(
      EmployeeLifecycleRequestSchema.safeParse({
        ...request,
        payload: { ...request.payload, reason },
      }).success,
      false,
    );
  }
});
