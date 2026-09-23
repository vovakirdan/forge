import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01a00000-0000-7000-8000-000000000001";
const other = "01a00000-0000-7000-8000-000000000002";
const pageId = "01a00000-0000-7000-8000-000000000003";
const entryId = "01a00000-0000-7000-8000-000000000004";
const stamp = "2026-09-23T10:00:00Z";
const signal = new AbortController().signal;
const page = {
  id: pageId,
  project_id: project,
  revision: 1,
  kind: "policy",
  status: "draft",
  content: { title: "Policy", markdown: "Text", source_refs: [] },
  content_hash: "a".repeat(64),
  actor: { kind: "human", id: pageId },
  command_id: pageId,
  created_at: stamp,
  revised_at: stamp,
  supersedes_revision: null,
};
const entry = {
  id: entryId,
  project_id: project,
  revision: 1,
  subject: { kind: "project_knowledge_entry", task_id: null },
  markdown: "Derived text",
  source_refs: [{ kind: "event", event_id: pageId }],
  content_hash: "b".repeat(64),
  coverage: null,
  created_by_job_id: pageId,
  created_at: stamp,
  withdrawn: false,
};

test("Knowledge reads use bounded after cursors and accept repeated IDs in revision history", async () => {
  const calls: string[] = [];
  const api = createLiveApi(async (input) => {
    const url = String(input);
    calls.push(url);
    if (url.includes("/history?"))
      return Response.json({ items: [page, { ...page, revision: 2 }], next_cursor: "2" });
    return Response.json({ items: [page], next_cursor: pageId });
  });
  assert.equal((await api.knowledgePages(project, null, "token", signal)).next_cursor, pageId);
  assert.equal((await api.knowledgeHistory(project, pageId, "1", "token", signal)).items.length, 2);
  assert.deepEqual(calls, [
    `/api/projects/${project}/knowledge?limit=20`,
    `/api/projects/${project}/knowledge/${pageId}/history?limit=20&after=1`,
  ]);
});

test("Memory list keeps project scope, source evidence and incomplete pages", async () => {
  const api = createLiveApi(async () => Response.json({ items: [entry], next_cursor: entryId }));
  const result = await api.memoryEntries(project, null, null, "token", signal);
  assert.equal(result.items[0]?.source_refs[0]?.kind, "event");
  assert.equal(result.next_cursor, entryId);
  const wrong = createLiveApi(async () =>
    Response.json({ items: [{ ...entry, project_id: other }], next_cursor: null }),
  );
  await assert.rejects(wrong.memoryEntries(project, null, null, "token", signal), {
    kind: "invalid_response",
  });
});

test("Search exposes canonical fallback and rejects foreign records or malformed scope", async () => {
  let calls = 0;
  const api = createLiveApi(async () => {
    calls += 1;
    return Response.json({
      project_id: project,
      employee_id: null,
      mode: "canonical_fallback",
      degradation: "not_configured",
      truncated: true,
      results: [
        { projection_id: null, score: null, document: { kind: "derived_memory", record: entry } },
      ],
    });
  });
  const result = await api.memorySearch(project, null, "derived", "token", signal);
  assert.equal(result.degradation, "not_configured");
  assert.equal(result.results[0]?.document.record.id, entryId);
  await assert.rejects(api.memorySearch(project, "../foreign", "derived", "token", signal), {
    kind: "invalid_id",
  });
  assert.equal(calls, 1);
  const foreign = createLiveApi(async () =>
    Response.json({
      ...result,
      results: [
        {
          ...result.results[0],
          document: { kind: "derived_memory", record: { ...entry, project_id: other } },
        },
      ],
    }),
  );
  await assert.rejects(foreign.memorySearch(project, null, "derived", "token", signal), {
    kind: "invalid_response",
  });
  const unpublished = createLiveApi(async () =>
    Response.json({
      ...result,
      results: [
        { projection_id: null, score: null, document: { kind: "knowledge_page", record: page } },
      ],
    }),
  );
  await assert.rejects(unpublished.memorySearch(project, null, "policy", "token", signal), {
    kind: "invalid_response",
  });
});

test("Invalid revision cursor is rejected before transport", async () => {
  let called = false;
  const api = createLiveApi(async () => {
    called = true;
    return Response.json({ items: [], next_cursor: null });
  });
  await assert.rejects(
    api.memoryHistory(project, entryId, "-1", "token", signal),
    (error: unknown) => error instanceof LiveApiError && error.kind === "cursor_invalid",
  );
  assert.equal(called, false);
});
