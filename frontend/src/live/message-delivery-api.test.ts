import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";
const project = "01988000-0000-7000-8000-000000000001";
const thread = "01988000-0000-7000-8000-000000000002";
const message = "01988000-0000-7000-8000-000000000003";

test("delivery read uses exact thread route and rejects mismatched waiver scope", async () => {
  const paths: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    paths.push(String(input));
    return Response.json({
      items: [
        {
          message_id: message,
          sequence: 2,
          assignment: null,
          runtime_accepted_at: null,
          acknowledged_at: null,
          answered_at: null,
          answered_reply_id: null,
          waiver: {
            message_id: message,
            project_id: thread,
            actor: { kind: "human", id: thread },
            reason: "obsolete",
            created_at: "2026-09-23T10:00:00Z",
          },
        },
      ],
    });
  };
  await assert.rejects(
    createLiveApi(fetcher).messageDelivery(
      project,
      thread,
      "1",
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
  assert.deepEqual(paths, [`/api/projects/${project}/threads/${thread}/delivery?limit=20&after=1`]);
});
