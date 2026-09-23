import assert from "node:assert/strict";
import { test } from "node:test";
import { knowledgeAttempt, reserveKnowledgePageId } from "../contracts/knowledge.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendKnowledgeCommand } from "./knowledge-command-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const pageId = "01988000-0000-7000-8000-000000000002";
const key = "01988000-0000-4000-8000-000000000003";
const signal = new AbortController().signal;
const receipt = {
  command_id: "01988000-0000-7000-8000-000000000004",
  status: "applied",
  project_revision: 4,
  event_ids: ["01988000-0000-7000-8000-000000000005"],
  resource: { kind: "knowledge_page", id: pageId },
};

test("new Knowledge page reserves UUIDv7 and freezes exact command body", () => {
  assert.match(
    reserveKnowledgePageId(1_790_000_000_000),
    /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  const attempt = knowledgeAttempt(
    "author_knowledge_page",
    {
      project_id: project,
      expected_revision: 3,
      payload: {
        page_id: pageId,
        expected_page_revision: 0,
        kind: "policy",
        title: "Policy",
        markdown: "Evidence",
        source_refs: [],
      },
    },
    key,
  );
  assert.equal(attempt.pageId, pageId);
  assert.equal(attempt.key, key);
  assert.equal(JSON.parse(attempt.body).payload.expected_page_revision, 0);
  assert.throws(() =>
    knowledgeAttempt(
      "publish_knowledge_page",
      {
        project_id: project,
        expected_revision: 3,
        payload: { page_id: pageId, expected_page_revision: 1, operation: "withdraw" },
      },
      key,
    ),
  );
});

test("retry sends the same bytes and key and accepts only correlated receipt", async () => {
  const attempt = knowledgeAttempt(
    "publish_knowledge_page",
    {
      project_id: project,
      expected_revision: 3,
      payload: { page_id: pageId, expected_page_revision: 1 },
    },
    key,
  );
  const calls: { url: string; body: BodyInit | null | undefined; key: string | null }[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    calls.push({
      url: String(input),
      body: init?.body,
      key: new Headers(init?.headers).get("Idempotency-Key"),
    });
    return Response.json(receipt);
  };
  const first = await sendKnowledgeCommand(
    fetcher,
    attempt,
    "token",
    signal,
    () => new Error("unauthorized"),
  );
  const second = await sendKnowledgeCommand(
    fetcher,
    attempt,
    "token",
    signal,
    () => new Error("unauthorized"),
  );
  assert.equal(first.command_id, second.command_id);
  assert.deepEqual(calls, [
    { url: "/api/commands/publish_knowledge_page", body: attempt.body, key },
    { url: "/api/commands/publish_knowledge_page", body: attempt.body, key },
  ]);
  const wrong = await assert.rejects(
    sendKnowledgeCommand(
      async () => Response.json({ ...receipt, resource: { kind: "task", id: pageId } }),
      attempt,
      "token",
      signal,
      () => new Error("unauthorized"),
    ),
  );
  void wrong;
});

test("invalid or changed frozen attempt is refused before transport", async () => {
  const attempt = knowledgeAttempt(
    "withdraw_knowledge_page",
    {
      project_id: project,
      expected_revision: 3,
      payload: { page_id: pageId, expected_page_revision: 1 },
    },
    key,
  );
  let called = false;
  await assert.rejects(
    sendKnowledgeCommand(
      async () => {
        called = true;
        return Response.json(receipt);
      },
      { ...attempt, body: `${attempt.body} ` },
      "token",
      signal,
      () => new Error("unauthorized"),
    ),
    (error: unknown) => error instanceof LiveCommandError && error.kind === "invalid_request",
  );
  assert.equal(called, false);
});
