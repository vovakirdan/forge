import assert from "node:assert/strict";
import { test } from "node:test";
import {
  createEmployeeAttempt,
  CreateEmployeeRequestSchema,
} from "../contracts/create-employee.ts";
import { sendCreateEmployee } from "./create-employee-api.ts";
import { LiveCommandError } from "./command-error.ts";

const request = CreateEmployeeRequestSchema.parse({
  project_id: "01988000-0000-7000-8000-000000000001",
  expected_revision: 3,
  payload: { name: "New teammate", role: "reviewer", stage_eligibility: { mode: "any" } },
});
const receipt = {
  command_id: "01988000-0000-7000-8000-000000000003",
  status: "applied",
  project_revision: 4,
  event_ids: ["01988000-0000-7000-8000-000000000004"],
  resource: { kind: "employee", id: "01988000-0000-7000-8000-000000000005" },
};

test("Employee command keeps the exact attempt and requires an Employee receipt", async () => {
  const attempt = createEmployeeAttempt(request, "11111111-1111-4111-8111-111111111111");
  const seen: { path?: string; body?: string; key?: string | null } = {};
  const fetcher = (async (path: string, init: RequestInit) => {
    seen.path = path;
    seen.body = init.body as string;
    seen.key = new Headers(init.headers).get("Idempotency-Key");
    return Response.json(receipt);
  }) as typeof fetch;
  const result = await sendCreateEmployee(
    fetcher,
    attempt,
    "a".repeat(64),
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  assert.deepEqual(result, receipt);
  assert.deepEqual(seen, {
    path: "/api/commands/create_employee",
    body: attempt.body,
    key: attempt.key,
  });
  for (const bad of [
    { ...receipt, resource: { kind: "task", id: receipt.resource.id } },
    { ...receipt, project_revision: 5 },
  ]) {
    const badFetcher = (async () => Response.json(bad)) as typeof fetch;
    await assert.rejects(
      sendCreateEmployee(
        badFetcher,
        attempt,
        "a".repeat(64),
        new AbortController().signal,
        () => new Error("unauthorized"),
      ),
      (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
    );
  }
});
