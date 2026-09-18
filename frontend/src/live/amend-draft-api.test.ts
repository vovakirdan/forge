import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi } from "./api.ts";
import { describeCommandError } from "./command-error.ts";
import { createDraftAttempt } from "./draft-attempt.ts";
import { ProjectViewSchema } from "../contracts/project.ts";
import { TaskDetailViewSchema } from "../contracts/task.ts";
import { ids, projectFixture, taskDetailFixture } from "../contracts/fixtures.ts";

const attempt = createDraftAttempt(
  {
    project: ProjectViewSchema.parse(projectFixture),
    task: TaskDetailViewSchema.parse({ ...taskDetailFixture, lifecycle: "draft" }),
  },
  { title: "Revised title", description: taskDetailFixture.description },
  "10000000-0000-4000-8000-000000000001",
);
const receipt = {
  command_id: ids.actor,
  status: "applied",
  project_revision: 4,
  event_ids: [ids.artifact],
  resource: { kind: "task", id: ids.task },
};
const signal = new AbortController().signal;

test("command uses one fixed POST and manual replay retains identical body and key", async () => {
  const calls: RequestInit[] = [];
  const api = createLiveApi(async (url, init) => {
    assert.equal(url, "/api/commands/amend_draft");
    calls.push(init ?? {});
    if (calls.length === 1) throw new Error("private network detail");
    return Response.json({ ...receipt, status: "replayed" });
  });
  await assert.rejects(api.amendDraft(attempt, "token", signal), { kind: "outcome_unknown" });
  assert.equal(calls.length, 1, "no hidden retry");
  assert.equal((await api.amendDraft(attempt, "token", signal)).status, "replayed");
  for (const call of calls) {
    assert.equal(call.method, "POST");
    assert.equal(call.body, attempt.body);
    assert.equal(call.credentials, "omit");
    assert.equal(call.redirect, "error");
    assert.equal(call.cache, "no-store");
    assert.equal(call.signal, signal);
    assert.equal(new Headers(call.headers).get("Idempotency-Key"), attempt.key);
    assert.equal(new Headers(call.headers).get("Authorization"), "Bearer token");
  }
});

test("only known status/code pairs are definite refusals and raw diagnostics are discarded", async () => {
  for (const [status, code, kind] of [
    [400, "invalid_request", "invalid_request"],
    [409, "stale_revision", "stale_revision"],
    [409, "idempotency_conflict", "idempotency_conflict"],
    [422, "validation_failed", "validation_failed"],
    [404, "not_found", "not_found"],
    [403, "forbidden", "forbidden"],
    [401, "anything", "unauthorized"],
    [500, "invalid_request", "outcome_unknown"],
    [409, "other_code", "outcome_unknown"],
    [422, "stale_revision", "outcome_unknown"],
    [400, "not_found", "outcome_unknown"],
  ] as const) {
    const api = createLiveApi(async () =>
      Response.json({ error: "private diagnostic", code }, { status }),
    );
    await assert.rejects(api.amendDraft(attempt, "token", signal), (error: unknown) => {
      assert.equal((error as { kind: string }).kind, kind);
      assert.equal(describeCommandError(error).includes("private"), false);
      assert.equal(String(error).includes("private"), false);
      return true;
    });
  }
});

test("invalid, oversized or incorrectly scoped receipts never prove a save", async () => {
  for (const value of [
    { ...receipt, resource: { kind: "task", id: ids.actor } },
    { ...receipt, resource: { kind: "project", id: ids.project } },
    { ...receipt, project_revision: 100 },
    { ...receipt, status: "queued" },
    { ...receipt, resource: undefined },
    { ...receipt, ignored: "x".repeat(66_000) },
  ]) {
    const api = createLiveApi(async () => Response.json(value));
    await assert.rejects(api.amendDraft(attempt, "token", signal), { kind: "outcome_unknown" });
  }
  for (const response of [
    new Response("not json"),
    Response.json(receipt, { status: 202 }),
    new Response(null, { status: 204 }),
    new Response("not json", { status: 400 }),
  ]) {
    await assert.rejects(createLiveApi(async () => response).amendDraft(attempt, "token", signal), {
      kind: "outcome_unknown",
    });
  }
});

test("aborted transport has an unknown command outcome, not a Core cancellation", async () => {
  const controller = new AbortController();
  controller.abort();
  const api = createLiveApi(async () => {
    throw new DOMException("cancelled", "AbortError");
  });
  await assert.rejects(api.amendDraft(attempt, "token", controller.signal), {
    kind: "outcome_unknown",
  });
});

test("malformed request identities cannot reach transport", async () => {
  let calls = 0;
  const api = createLiveApi(async () => {
    calls++;
    return Response.json(receipt);
  });
  await assert.rejects(api.amendDraft({ ...attempt, taskId: ids.actor }, "token", signal), {
    kind: "invalid_request",
  });
  await assert.rejects(api.amendDraft({ ...attempt, key: "bad" }, "token", signal));
  assert.equal(calls, 0);
});
