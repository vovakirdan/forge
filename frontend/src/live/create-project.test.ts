import assert from "node:assert/strict";
import { test } from "node:test";
import { createProjectAttempt, reserveProjectId } from "../contracts/create-project.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendCreateProject } from "./create-project-api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const key = "01988000-0000-4000-8000-000000000002";

test("Project creation reserves UUIDv7 and freezes zero revision command", () => {
  assert.match(
    reserveProjectId(1_790_000_000_000),
    /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  const attempt = createProjectAttempt(project, "New Project", key);
  assert.deepEqual(JSON.parse(attempt.body), {
    project_id: project,
    expected_revision: 0,
    payload: { name: "New Project" },
  });
  assert.throws(() => createProjectAttempt(project, " ", key));
});

test("Project retry repeats exact body and key and rejects a foreign receipt", async () => {
  const attempt = createProjectAttempt(project, "New Project", key);
  const calls: { url: string; body: BodyInit | null | undefined; key: string | null }[] = [];
  const receipt = {
    command_id: "01988000-0000-7000-8000-000000000003",
    status: "applied",
    project_revision: 1,
    event_ids: ["01988000-0000-7000-8000-000000000004"],
    resource: { kind: "project", id: project },
  };
  const fetcher: typeof fetch = async (input, init) => {
    calls.push({
      url: String(input),
      body: init?.body,
      key: new Headers(init?.headers).get("Idempotency-Key"),
    });
    return Response.json(receipt);
  };
  await sendCreateProject(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  await sendCreateProject(
    fetcher,
    attempt,
    "token",
    new AbortController().signal,
    () => new Error("unauthorized"),
  );
  assert.deepEqual(calls, [
    { url: "/api/commands/create_project", body: attempt.body, key },
    { url: "/api/commands/create_project", body: attempt.body, key },
  ]);
  await assert.rejects(
    sendCreateProject(
      async () =>
        Response.json({
          ...receipt,
          resource: { kind: "project", id: key.replace("4000", "7000") },
        }),
      attempt,
      "token",
      new AbortController().signal,
      () => new Error("unauthorized"),
    ),
    (error: unknown) => error instanceof LiveCommandError && error.kind === "outcome_unknown",
  );
});
