import { z } from "zod";
import {
  ActorReferenceSchema,
  RevisionSchema,
  StableKeySchema,
  TimestampSchema,
  UuidV7Schema,
} from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

const SequenceSchema = z.number().int().nonnegative().safe();
export const TaskContextSchema = z.object({
  task_id: UuidV7Schema,
  pipeline_version_id: UuidV7Schema,
  stage_id: StableKeySchema,
  stage_visit: RevisionSchema,
});
export const MessageTargetSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("inbox") }).passthrough(),
  z.object({ kind: z.literal("task_execution"), context: TaskContextSchema }).passthrough(),
  z
    .object({
      kind: z.literal("exact_run"),
      context: TaskContextSchema,
      run_id: UuidV7Schema,
      fencing_token: RevisionSchema,
      environment_epoch: RevisionSchema,
    })
    .passthrough(),
]);

export const EmployeeThreadSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      project_id: UuidV7Schema,
      employee_id: UuidV7Schema,
      task_id: UuidV7Schema.nullable(),
      created_by: ActorReferenceSchema,
      created_at: TimestampSchema,
      updated_at: TimestampSchema,
      revision: RevisionSchema,
      last_sequence: SequenceSchema,
    })
    .passthrough(),
);

export const EmployeeMessageSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      thread_id: UuidV7Schema,
      project_id: UuidV7Schema,
      employee_id: UuidV7Schema,
      sequence: RevisionSchema,
      sender: ActorReferenceSchema,
      target: MessageTargetSchema,
      kind: z.enum(["instruction", "question", "notification", "reply"]),
      requirement: z.enum(["informational", "acknowledged", "answered"]),
      body: z.string().min(1),
      reply_to: UuidV7Schema.nullable(),
      created_at: TimestampSchema,
    })
    .passthrough(),
);

export const EmployeeThreadListSchema = preserveWireValue(
  z
    .object({
      items: z.array(EmployeeThreadSchema).max(20),
      next_cursor: UuidV7Schema.optional(),
    })
    .passthrough(),
);

export const EmployeeMessageListSchema = preserveWireValue(
  z
    .object({
      items: z.array(EmployeeMessageSchema).max(20),
      next_cursor: z
        .string()
        .regex(/^(0|[1-9][0-9]*)$/)
        .refine((value) => BigInt(value) <= BigInt(Number.MAX_SAFE_INTEGER))
        .optional(),
    })
    .passthrough(),
);

export const MessageDeliverySchema = z
  .object({
    message_id: UuidV7Schema,
    sequence: RevisionSchema,
    assignment: z
      .object({
        state: z.enum(["queued", "leased", "completed", "held"]),
        attempt_number: z.number().int().nonnegative().safe(),
        run_id: UuidV7Schema.nullable(),
        retry_ready: z.boolean(),
      })
      .nullable(),
    runtime_accepted_at: TimestampSchema.nullable(),
    acknowledged_at: TimestampSchema.nullable(),
    answered_at: TimestampSchema.nullable(),
    answered_reply_id: UuidV7Schema.nullable(),
    waiver: z
      .object({
        message_id: UuidV7Schema,
        project_id: UuidV7Schema,
        actor: ActorReferenceSchema,
        reason: z.string().min(1).max(4096),
        created_at: TimestampSchema,
      })
      .nullable(),
  })
  .superRefine((item, ctx) => {
    if ((item.answered_at === null) !== (item.answered_reply_id === null))
      ctx.addIssue({ code: z.ZodIssueCode.custom, message: "Answer receipt and reply differ" });
  });
export const MessageDeliveryListSchema = z.object({
  items: z.array(MessageDeliverySchema).max(20),
  next_cursor: z
    .string()
    .regex(/^(0|[1-9][0-9]*)$/)
    .refine((value) => BigInt(value) <= BigInt(Number.MAX_SAFE_INTEGER))
    .optional(),
});
export type MessageDelivery = z.infer<typeof MessageDeliverySchema>;

export type EmployeeThread = z.infer<typeof EmployeeThreadSchema>;
export type EmployeeMessage = z.infer<typeof EmployeeMessageSchema>;
