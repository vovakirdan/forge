import assert from "node:assert/strict";
import { test } from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const task = "01988000-0000-7000-8000-000000000002";
const foreign = "01988000-0000-7000-8000-000000000003";

test("surface reads use only exact scoped routes and bounded pages", async () => {
  const paths: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    paths.push(String(input));
    return Response.json(
      String(input).includes("git-source-policy")
        ? { revision: 1, policy: { mode: "latest_target" } }
        : { items: [] },
    );
  };
  const api = createLiveApi(fetcher);
  await api.taskSurface(
    project,
    task,
    "git-source-policy",
    null,
    "token",
    new AbortController().signal,
  );
  await api.taskSurface(project, task, "reviews", foreign, "token", new AbortController().signal);
  assert.deepEqual(paths, [
    `/api/projects/${project}/tasks/${task}/git-source-policy`,
    `/api/projects/${project}/tasks/${task}/reviews?limit=20&after=${foreign}`,
  ]);
});

test("snapshot scope mismatch is rejected", async () => {
  const fetcher: typeof fetch = async () =>
    Response.json({
      items: [
        {
          id: foreign,
          artifact_id: foreign,
          task_id: foreign,
          title: "x",
          state: "pending",
          error_code: null,
          manifest: null,
          created_at: "2026-09-23T10:00:00Z",
        },
      ],
    });
  await assert.rejects(
    createLiveApi(fetcher).taskSurface(
      project,
      task,
      "file-snapshots",
      null,
      "token",
      new AbortController().signal,
    ),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});
