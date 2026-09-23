import assert from "node:assert/strict";
import { test } from "node:test";
import {
  pipelineManagementAttempt,
  PipelineDefinitionInputSchema,
} from "../contracts/pipeline-management.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendPipelineManagementCommand } from "./pipeline-management-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const pipeline = "01988000-0000-7000-8000-000000000002";
const version = "01988000-0000-7000-8000-000000000003";
const key = "01988000-0000-4000-8000-000000000004";
const definition = {
  task_kinds: ["delivery"],
  entry_stage_id: "work",
  stages: [{ id: "work", name: "Work", executor_kind: "employee", outcomes: ["done"] }],
  transitions: [{ from_stage_id: "work", outcome: "done", target: { kind: "done" } }],
};

test("Pipeline publication freezes full graph and rejects unsupported fields", () => {
  const attempt = pipelineManagementAttempt(
    "publish_pipeline_version",
    {
      project_id: project,
      expected_revision: 3,
      payload: {
        pipeline_id: pipeline,
        expected_pipeline_revision: 2,
        definition,
        make_default: false,
      },
    },
    key,
  );
  assert.equal(attempt.resourceKind, "pipeline_version");
  assert.equal(JSON.parse(attempt.body).payload.definition.stages.length, 1);
  assert.throws(() =>
    PipelineDefinitionInputSchema.parse({
      ...definition,
      stages: [{ ...definition.stages[0], actor: "forged" }],
    }),
  );
});

test("Pipeline commands retry exact bytes and accept correlated receipt", async () => {
  const attempt = pipelineManagementAttempt(
    "set_pipeline_default_version",
    {
      project_id: project,
      expected_revision: 3,
      payload: {
        pipeline_id: pipeline,
        expected_pipeline_revision: 2,
        pipeline_version_id: version,
      },
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
    return Response.json({
      command_id: version,
      status: "applied",
      project_revision: 4,
      event_ids: [version],
      resource: { kind: "pipeline", id: pipeline },
    });
  };
  await sendPipelineManagementCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  await sendPipelineManagementCommand(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  assert.deepEqual(calls, [
    { url: "/api/commands/set_pipeline_default_version", body: attempt.body, key },
    { url: "/api/commands/set_pipeline_default_version", body: attempt.body, key },
  ]);
  await assert.rejects(
    sendPipelineManagementCommand(
      async () =>
        Response.json({
          command_id: version,
          status: "applied",
          project_revision: 4,
          event_ids: [version],
          resource: { kind: "task", id: pipeline },
        }),
      attempt,
      "token",
      new AbortController().signal,
      () => new Error("unauthorized"),
    ),
    (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});
