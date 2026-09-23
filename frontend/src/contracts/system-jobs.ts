import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { EmployeeOnboardingStatusSchema } from "./knowledge.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

const CountSchema = z.number().int().nonnegative().safe();
const PolicySchema = preserveWireValue(
  z
    .object({
      enabled: z.boolean(),
      max_concurrent: RevisionSchema,
      max_attempts_per_job: RevisionSchema,
      max_attempts_per_day: RevisionSchema,
      max_input_bytes: RevisionSchema,
      max_result_bytes: RevisionSchema,
      wall_seconds: RevisionSchema,
      coalesce_seconds: CountSchema,
    })
    .passthrough(),
);

export const SystemJobSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      project_id: UuidV7Schema,
      kind: z.enum(["summarization", "onboarding"]),
      source_task_id: UuidV7Schema.nullable(),
      target_employee_id: UuidV7Schema.nullable(),
      generation: RevisionSchema,
      covered_sequence: CountSchema,
      state: z.enum(["pending", "running", "completed", "held", "cancelled"]),
      reason_code: z.string().nullable(),
    })
    .passthrough(),
);

export const SystemJobStatusSchema = preserveWireValue(
  z
    .object({
      policy: PolicySchema.nullable(),
      settings_revision: RevisionSchema.nullable(),
      jobs: z.array(SystemJobSchema).max(256),
      usage: z
        .object({
          attempts_last_24h: CountSchema,
          running: CountSchema,
          usage_status: z.string(),
        })
        .passthrough(),
      onboarding: z.array(EmployeeOnboardingStatusSchema),
    })
    .passthrough(),
);

export const SystemJobAttemptSchema = z
  .object({
    id: UuidV7Schema,
    job_id: UuidV7Schema,
    generation: RevisionSchema,
    run_id: UuidV7Schema,
    state: z.enum(["running", "completed", "stopped", "failed", "superseded"]),
    result_accepted: z.boolean(),
    result_message_id: UuidV7Schema.nullable(),
    created_at: z.string().datetime({ offset: true }),
    completed_at: z.string().datetime({ offset: true }).nullable(),
    materialized_entries: z
      .array(z.object({ id: UuidV7Schema, revision: RevisionSchema }).strict())
      .max(18),
  })
  .strict()
  .refine((attempt) => attempt.result_accepted === (attempt.result_message_id !== null), {
    message: "result acceptance requires its durable message coordinate",
  });

export const SystemJobAttemptPageSchema = z
  .object({
    items: z.array(SystemJobAttemptSchema).max(50),
    next_cursor: UuidV7Schema.optional(),
  })
  .strict();

export type SystemJobAttemptPage = z.infer<typeof SystemJobAttemptPageSchema>;

export type SystemJobStatus = z.infer<typeof SystemJobStatusSchema>;
export type EmployeeOnboardingStatus = z.infer<typeof EmployeeOnboardingStatusSchema>;
export { EmployeeOnboardingStatusSchema };
