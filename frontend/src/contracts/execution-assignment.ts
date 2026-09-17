import { z } from "zod";
import { StableKeySchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

const positiveSafeInteger = z.number().int().positive().safe();
export const SystemJobKindSchema = z.enum(["summarization", "onboarding"]);

// The purpose-specific owner is explicit. Contextual Task IDs do not grant
// Task-stage ownership, and a SystemJob is not an Employee assignment.
export const ExecutionAssignmentSchema = preserveWireValue(
  z.discriminatedUnion("purpose", [
    z
      .object({
        purpose: z.literal("task_stage"),
        owner: z
          .object({
            task_id: UuidV7Schema,
            queue_entry_id: UuidV7Schema,
            stage_id: StableKeySchema,
          })
          .passthrough(),
      })
      .passthrough(),
    z
      .object({
        purpose: z.literal("communication"),
        owner: z
          .object({
            assignment_id: UuidV7Schema,
            thread_id: UuidV7Schema,
            source_message_id: UuidV7Schema,
          })
          .passthrough(),
      })
      .passthrough(),
    z
      .object({
        purpose: z.literal("resolution"),
        owner: z
          .object({
            assignment_id: UuidV7Schema,
            escalation_id: UuidV7Schema,
            lease_generation: positiveSafeInteger,
          })
          .passthrough(),
      })
      .passthrough(),
    z
      .object({
        purpose: z.literal("hook"),
        owner: z
          .object({
            invocation_id: UuidV7Schema,
            task_id: UuidV7Schema,
            pipeline_version_id: UuidV7Schema,
            stage_id: StableKeySchema,
            stage_visit: positiveSafeInteger,
            candidate_proposal_id: UuidV7Schema,
          })
          .passthrough(),
      })
      .passthrough(),
    z
      .object({
        purpose: z.literal("system_job"),
        owner: z
          .object({
            job_id: UuidV7Schema,
            attempt_id: UuidV7Schema,
            generation: positiveSafeInteger,
            kind: SystemJobKindSchema,
          })
          .passthrough(),
      })
      .passthrough(),
  ]),
);

export type SystemJobKind = z.infer<typeof SystemJobKindSchema>;
export type ExecutionAssignment = z.infer<typeof ExecutionAssignmentSchema>;
