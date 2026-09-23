import assert from "node:assert/strict";
import test from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const pipeline = "01988000-0000-7000-8000-000000000002";

test("catalog read uses scoped cursor and retains separate default/latest", async () => {
  const api = createLiveApi(async (input) => {
    assert.equal(
      String(input),
      `/api/projects/${project}/pipeline-catalog?limit=20&cursor=${pipeline}`,
    );
    return Response.json({
      items: [
        {
          id: pipeline,
          name: "Delivery",
          revision: 3,
          default_version_id: "01988000-0000-7000-8000-000000000003",
          latest_version_id: "01988000-0000-7000-8000-000000000004",
          latest_version: 2,
          deleted_at: null,
          pinned_task_count: 1,
        },
      ],
    });
  });
  const page = await api.pipelineCatalog(project, pipeline, "token", new AbortController().signal);
  assert.notEqual(page.items[0]?.default_version_id, page.items[0]?.latest_version_id);
});

test("catalog rejects duplicate Pipeline identities", async () => {
  const item = {
    id: pipeline,
    name: "Delivery",
    revision: 1,
    default_version_id: null,
    latest_version_id: null,
    latest_version: null,
    deleted_at: null,
    pinned_task_count: 0,
  };
  const api = createLiveApi(async () => Response.json({ items: [item, item] }));
  await assert.rejects(
    api.pipelineCatalog(project, null, "token", new AbortController().signal),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});
