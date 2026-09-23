import assert from "node:assert/strict";
import { test } from "node:test";
import { ids, timestamp } from "../contracts/fixtures.ts";
import { createLiveApi } from "./api.ts";

const safe = {
  availability: "available",
  employee_id: ids.actor,
  binding_revision: 2,
  updated_at: timestamp,
  profile_id: ids.artifact,
  profile_revision: 1,
  adapter_id: "codex_cli",
  adapter_version: "0.153.2",
  provider_id: "openai",
  model: "gpt-5.6-luna",
  credential_binding_id: ids.wait1,
  credential_delivery: "isolated_runtime_secret",
  image_digest: "a".repeat(64),
  surface_kind: "none",
  access: "read_write",
  limits: {
    cpu_millis: 1000,
    memory_bytes: 1_073_741_824,
    pids: 100,
    wall_seconds: 600,
    stop_grace_seconds: 15,
  },
  budget: {
    max_output_bytes: 1024,
    requests_per_minute: 60,
    tokens_per_minute: 1000,
    max_spend_microusd: null,
  },
  prompts_available: true,
};

test("Employee runtime metadata read is scoped and safe", async () => {
  const paths: string[] = [];
  const api = createLiveApi(async (url) => {
    paths.push(String(url));
    return Response.json(safe);
  });
  assert.deepEqual(
    await api.employeeRuntimeMetadata(
      ids.project,
      ids.actor,
      "token",
      new AbortController().signal,
    ),
    safe,
  );
  assert.deepEqual(paths, [`/api/projects/${ids.project}/employees/${ids.actor}/runtime-metadata`]);
});

test("Employee runtime metadata rejects prompt, secret and foreign Employee fields", async () => {
  for (const value of [
    { ...safe, system_prompt: "private" },
    { ...safe, credential_secret_id: ids.wait2 },
    { ...safe, employee_id: ids.wait2 },
  ]) {
    const api = createLiveApi(async () => Response.json(value));
    await assert.rejects(
      api.employeeRuntimeMetadata(ids.project, ids.actor, "token", new AbortController().signal),
      { kind: "invalid_response" },
    );
  }
});
