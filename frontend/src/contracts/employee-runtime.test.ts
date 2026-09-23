import { test } from "node:test";
import assert from "node:assert/strict";
import { configureEmployeeRuntimeAttempt } from "./employee-runtime.ts";

const project = "01988000-0000-7000-8000-000000000001";
const employee = "01988000-0000-7000-8000-000000000002";
const profile = "01988000-0000-7000-8000-000000000003";
const credential = "01988000-0000-7000-8000-000000000004";
const secret = "01988000-0000-7000-8000-000000000005";
const binding = {
  execution_profile: {
    id: profile,
    revision: 1,
    project_id: project,
    adapter_id: "opencode_runtime",
    adapter_version: "1",
    provider_id: "openai",
    model: "test",
    credential_binding: {
      id: credential,
      project_id: project,
      secret_id: secret,
      account_id: null,
      allowed_delivery_modes: ["proxy_only"],
    },
    credential_delivery: "proxy_only",
    capability_profile: {
      adapter_id: "opencode_runtime",
      adapter_version: "1",
      transport_engine: "api_runtime",
      capabilities: ["controlled_stop", "gateway_auth"],
      credential_exposed_to_run: false,
    },
  },
  image: `runner@sha256:${"a".repeat(64)}`,
  surface: { mode: "none" },
  system_prompt: "System",
  employee_prompt: "Employee",
};
test("runtime attempt accepts credential references and rejects secret bytes", () => {
  const attempt = configureEmployeeRuntimeAttempt(project, employee, 3, binding);
  assert.equal(
    JSON.parse(attempt.body).payload.binding.execution_profile.credential_binding.secret_id,
    secret,
  );
  assert.throws(() =>
    configureEmployeeRuntimeAttempt(project, employee, 3, { ...binding, api_key: "secret bytes" }),
  );
  assert.throws(() =>
    configureEmployeeRuntimeAttempt(project, employee, 3, {
      ...binding,
      execution_profile: { ...binding.execution_profile, project_id: employee },
    }),
  );
});
