import assert from "node:assert/strict";
import test from "node:test";
import { parseEventBatch, readProjectEvents } from "./event-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const eventId = "01988000-0000-7000-8000-000000000002";
const event = (sequence: number, projectId = project) =>
  `id: ${sequence}\nevent: forge.event\ndata: ${JSON.stringify({ schema_version: 1, event_id: eventId, project_id: projectId, project_sequence: sequence, event_type: "task.created", occurred_at: "2026-09-23T00:00:00Z" })}\n\n`;

test("SSE replay accepts a scoped ordered batch and rejects duplicates or foreign events", () => {
  assert.deepEqual(
    parseEventBatch(event(2) + event(3), project, 1).map((item) => item.project_sequence),
    [2, 3],
  );
  assert.throws(() => parseEventBatch(event(2), project, 2));
  assert.throws(() => parseEventBatch(event(3) + event(2), project, 1));
  assert.throws(() => parseEventBatch(event(2, eventId), project, 1));
  assert.throws(() => parseEventBatch("id: 2\nevent: forge.event\ndata: {}\n\n", project, 1));
});

test("event fetch sends bearer only to the scoped gateway path", async () => {
  const calls: string[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    calls.push(String(input));
    assert.equal(init?.credentials, "omit");
    assert.equal(new Headers(init?.headers).get("Authorization"), "Bearer token");
    return new Response(event(4), { headers: { "Content-Type": "text/event-stream" } });
  };
  const batch = await readProjectEvents(fetcher, project, 3, "token", new AbortController().signal);
  assert.equal(batch[0]?.project_sequence, 4);
  assert.deepEqual(calls, [`/api/projects/${project}/events?after=3`]);
});
