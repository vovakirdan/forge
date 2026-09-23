import { strict as assert } from "node:assert";
import { describe, it } from "node:test";
import { gitRecoveryAttempt } from "./git-recovery-command.ts";

const project = "01988000-0000-7000-8000-000000000001";
const operation = "01988000-0000-7000-8000-000000000002";
const request = {
  project_id: project,
  expected_revision: 5,
  payload: {
    operation_id: operation,
    expected_task_revision: 9,
    reason: "Reviewed held integration",
  },
};

describe("Git integration recovery command", () => {
  it("freezes exact retry and accept attempts", () => {
    for (const action of ["retry_git_integration", "accept_git_integration_result"] as const) {
      const attempt = gitRecoveryAttempt(action, request, "f862c4a2-6cd0-4b77-9cf8-554a3c551197");
      assert.equal(Object.isFrozen(attempt), true);
      assert.deepEqual(JSON.parse(attempt.body), request);
      assert.equal(attempt.operationId, operation);
    }
  });

  it("rejects blank reasons and extra authority fields", () => {
    assert.throws(() =>
      gitRecoveryAttempt("retry_git_integration", {
        ...request,
        payload: { ...request.payload, reason: "   " },
      }),
    );
    assert.throws(() =>
      gitRecoveryAttempt("accept_git_integration_result", {
        ...request,
        payload: { ...request.payload, force: true },
      } as never),
    );
  });
});
