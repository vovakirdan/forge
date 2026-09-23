import assert from "node:assert/strict";
import { test } from "node:test";
import { gitRecoveryAttempt } from "../contracts/git-recovery-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendGitRecoveryCommand } from "./git-recovery-command-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const operation = "01988000-0000-7000-8000-000000000002";
const event = "01988000-0000-7000-8000-000000000003";
const key = "550e8400-e29b-41d4-a716-446655440000";
const attempt = gitRecoveryAttempt(
  "accept_git_integration_result",
  {
    project_id: project,
    expected_revision: 5,
    payload: {
      operation_id: operation,
      expected_task_revision: 9,
      reason: "Recorded Applied result",
    },
  },
  key,
);

test("accept recorded Git result preserves exact body and validates replay receipt", async () => {
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
      project_revision: 6,
      event_ids: [event],
      resource: { kind: "git_integration", id: operation },
    });
  };
  const receipt = await sendGitRecoveryCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => Error("unauthorized"),
  );
  assert.equal(receipt.status, "replayed");
  assert.deepEqual(seen, [
    { path: "/api/commands/accept_git_integration_result", body: attempt.body, key },
  ]);
});

test("wrong Git integration receipt leaves the outcome unknown", async () => {
  const fetcher: typeof fetch = async () =>
    Response.json({
      command_id: event,
      status: "applied",
      project_revision: 6,
      event_ids: [event],
      resource: { kind: "git_integration", id: project },
    });
  await assert.rejects(
    sendGitRecoveryCommand(fetcher, attempt, "token", new AbortController().signal, () =>
      Error("unauthorized"),
    ),
    (error) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});
