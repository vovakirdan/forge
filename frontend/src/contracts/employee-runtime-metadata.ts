import { z } from "zod";
import { RevisionSchema, TimestampSchema, UuidV7Schema } from "./common.ts";

const Available = z
  .object({
    availability: z.literal("available"),
    employee_id: UuidV7Schema,
    binding_revision: RevisionSchema,
    updated_at: TimestampSchema,
    profile_id: UuidV7Schema,
    profile_revision: RevisionSchema,
    adapter_id: z.string().min(1),
    adapter_version: z.string().min(1),
    provider_id: z.string().min(1),
    model: z.string().min(1),
    credential_binding_id: UuidV7Schema,
    credential_delivery: z.enum(["proxy_only", "isolated_runtime_secret", "trusted_host"]),
    image_digest: z.string().regex(/^[0-9a-f]{64}$/),
    surface_kind: z.enum([
      "none",
      "filesystem_sandbox",
      "git_worktree",
      "git_unborn",
      "git_unborn_candidate_snapshot",
      "git_candidate_snapshot",
    ]),
    access: z.enum(["read_write", "read_only"]),
    limits: z
      .object({
        cpu_millis: z.number().int().positive().safe(),
        memory_bytes: z.number().int().positive().safe(),
        pids: z.number().int().positive().safe(),
        wall_seconds: z.number().int().positive().safe(),
        stop_grace_seconds: z.number().int().positive().safe(),
      })
      .strict(),
    budget: z
      .object({
        max_output_bytes: z.number().int().positive().safe(),
        requests_per_minute: z.number().int().positive().safe(),
        tokens_per_minute: z.number().int().positive().safe(),
        max_spend_microusd: z.number().int().positive().safe().nullable(),
      })
      .strict(),
    prompts_available: z.literal(true),
  })
  .strict();

export const EmployeeRuntimeMetadataSchema = z.discriminatedUnion("availability", [
  Available,
  z.object({ availability: z.literal("unavailable"), employee_id: UuidV7Schema }).strict(),
]);
