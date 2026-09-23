import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const positive = z.number().int().positive().safe();
const nonnegative = z.number().int().nonnegative().safe();
export const SystemJobPolicyInputSchema = z
  .object({
    enabled: z.boolean(),
    max_concurrent: positive.max(16),
    max_attempts_per_job: positive.max(10),
    max_attempts_per_day: positive.max(100_000),
    max_input_bytes: positive.min(1024).max(256 * 1024),
    max_result_bytes: positive.min(1024).max(64 * 1024),
    wall_seconds: positive.min(10).max(3600),
    coalesce_seconds: nonnegative.max(300),
  })
  .strict();

const credentialBinding = z
  .object({
    id: UuidV7Schema,
    project_id: UuidV7Schema,
    secret_id: UuidV7Schema,
    account_id: z.string().nullable(),
    allowed_delivery_modes: z.array(
      z.enum(["proxy_only", "isolated_runtime_secret", "trusted_host"]),
    ),
  })
  .strict();
const capability = z
  .object({
    adapter_id: z.string(),
    adapter_version: z.string(),
    transport_engine: z.enum(["acp", "cli_wrapper", "api_runtime"]),
    capabilities: z.array(
      z.enum([
        "structured_events",
        "session_resume",
        "model_selection",
        "usage_reporting",
        "controlled_stop",
        "gateway_auth",
        "native_mcp",
        "live_input",
      ]),
    ),
    credential_exposed_to_run: z.boolean(),
  })
  .strict();
const executionProfile = z
  .object({
    id: UuidV7Schema,
    revision: positive,
    project_id: UuidV7Schema,
    adapter_id: z.string(),
    adapter_version: z.string(),
    provider_id: z.string(),
    model: z.string(),
    credential_binding: credentialBinding,
    credential_delivery: z.enum(["proxy_only", "isolated_runtime_secret", "trusted_host"]),
    capability_profile: capability,
  })
  .strict();
export const SystemJobBindingInputSchema = z
  .object({
    execution_profile: executionProfile,
    image: z.string().regex(/^.+@sha256:[0-9a-fA-F]{64}$/),
    surface: z.object({ mode: z.literal("none") }).strict(),
    access: z.enum(["read_only", "read_write"]).optional(),
    limits: z
      .object({
        cpu_millis: positive,
        memory_bytes: positive,
        pids: positive,
        wall_seconds: positive,
        stop_grace_seconds: positive,
      })
      .strict()
      .optional(),
    budget: z
      .object({
        max_output_bytes: positive,
        requests_per_minute: positive,
        tokens_per_minute: positive,
        max_spend_microusd: positive.nullable(),
      })
      .strict()
      .optional(),
    system_prompt: z.string().min(1),
    employee_prompt: z.string().min(1),
  })
  .strict();

const payloads = {
  configure_system_jobs: z
    .object({
      expected_settings_revision: nonnegative,
      policy: SystemJobPolicyInputSchema,
      binding: SystemJobBindingInputSchema,
    })
    .strict(),
  request_task_summary: z.object({ task_id: UuidV7Schema }).strict(),
  request_employee_onboarding: z.object({ employee_id: UuidV7Schema }).strict(),
  retry_system_job: z.object({ job_id: UuidV7Schema }).strict(),
  skip_employee_onboarding: z
    .object({
      employee_id: UuidV7Schema,
      reason: z
        .string()
        .refine(
          (value) => value.trim().length > 0 && new TextEncoder().encode(value).length <= 2000,
        ),
    })
    .strict(),
};
export const SystemJobActionSchema = z.enum([
  "configure_system_jobs",
  "request_task_summary",
  "request_employee_onboarding",
  "retry_system_job",
  "skip_employee_onboarding",
]);
export type SystemJobAction = z.infer<typeof SystemJobActionSchema>;
export type SystemJobAttempt = Readonly<{
  action: SystemJobAction;
  body: string;
  key: string;
  projectId: string;
  expectedProjectRevision: number;
  resourceKind: "system_job" | "employee" | null;
  expectedResourceId: string | null;
}>;

export function systemJobAttempt(
  action: SystemJobAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): SystemJobAttempt {
  const selected = SystemJobActionSchema.parse(action);
  const projectId = UuidV7Schema.parse(request.project_id);
  const expected = RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1).parse(request.expected_revision);
  const payload = payloads[selected].parse(request.payload);
  if (
    selected === "configure_system_jobs" &&
    (
      payload as { binding: z.infer<typeof SystemJobBindingInputSchema> }
    ).binding.execution_profile.project_id.toLowerCase() !== projectId.toLowerCase()
  )
    throw new Error("Runtime binding belongs to another Project");
  const resourceKind =
    selected === "configure_system_jobs"
      ? null
      : selected === "skip_employee_onboarding"
        ? ("employee" as const)
        : ("system_job" as const);
  const expectedResourceId =
    selected === "retry_system_job"
      ? (payload as { job_id: string }).job_id
      : selected === "skip_employee_onboarding"
        ? (payload as { employee_id: string }).employee_id
        : null;
  return Object.freeze({
    action: selected,
    body: JSON.stringify({ project_id: projectId, expected_revision: expected, payload }),
    key: IdempotencyKeySchema.parse(key),
    projectId,
    expectedProjectRevision: expected,
    resourceKind,
    expectedResourceId,
  });
}

export const SystemJobReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z
      .object({ kind: z.enum(["system_job", "employee"]), id: UuidV7Schema })
      .strict()
      .optional(),
  })
  .strict();
