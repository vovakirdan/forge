import assert from "node:assert/strict";
import { test } from "node:test";
import { createPipelineAttempt, InitialPipelineDefinition } from "../contracts/create-pipeline.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendCreatePipeline } from "./create-pipeline-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const pipeline = "01988000-0000-7000-8000-000000000002";
const key = "01988000-0000-4000-8000-000000000003";

test("create Pipeline freezes complete first graph under Project revision", () => {
  const attempt = createPipelineAttempt(project, 3, "Delivery", InitialPipelineDefinition, key);
  assert.equal(JSON.parse(attempt.body).payload.name, "Delivery");
  assert.equal(JSON.parse(attempt.body).payload.stages[0].id, "work");
  assert.throws(() =>
    createPipelineAttempt(
      project,
      3,
      "Delivery",
      { ...InitialPipelineDefinition, actor: "forged" },
      key,
    ),
  );
});

test("create Pipeline replays identical bytes and checks generated catalog receipt", async () => {
  const attempt = createPipelineAttempt(project, 3, "Delivery", InitialPipelineDefinition, key);
  const receipt = {
    command_id: "01988000-0000-7000-8000-000000000004",
    status: "applied",
    project_revision: 4,
    event_ids: ["01988000-0000-7000-8000-000000000005", "01988000-0000-7000-8000-000000000006"],
    resource: { kind: "pipeline", id: pipeline },
  };
  const calls: { url: string; body: BodyInit | null | undefined; key: string | null }[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    calls.push({
      url: String(input),
      body: init?.body,
      key: new Headers(init?.headers).get("Idempotency-Key"),
    });
    return Response.json(receipt);
  };
  await sendCreatePipeline(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  await sendCreatePipeline(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  assert.deepEqual(calls, [
    { url: "/api/commands/create_pipeline", body: attempt.body, key },
    { url: "/api/commands/create_pipeline", body: attempt.body, key },
  ]);
  await assert.rejects(
    sendCreatePipeline(
      async () =>
        Response.json({ ...receipt, resource: { kind: "pipeline_version", id: pipeline } }),
      attempt,
      "token",
      new AbortController().signal,
      () => new Error("unauthorized"),
    ),
    (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});
