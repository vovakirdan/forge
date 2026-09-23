import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

export const InboxActionSchema = z.enum([
  "open_employee_thread",
  "send_employee_message",
  "waive_message_requirement",
  "retry_communication",
]);
export type InboxAction = z.infer<typeof InboxActionSchema>;
const OpenPayload = z
  .object({ employee_id: UuidV7Schema, task_id: UuidV7Schema.nullable() })
  .strict();
const Context = z
  .object({
    task_id: UuidV7Schema,
    pipeline_version_id: UuidV7Schema,
    stage_id: StableKeySchema,
    stage_visit: RevisionSchema,
  })
  .strict();
const CommandTarget = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("inbox") }).strict(),
  z.object({ kind: z.literal("task_execution"), context: Context }).strict(),
  z
    .object({
      kind: z.literal("exact_run"),
      context: Context,
      run_id: UuidV7Schema,
      fencing_token: RevisionSchema,
      environment_epoch: RevisionSchema,
    })
    .strict(),
]);
const SendPayload = z
  .object({
    thread_id: UuidV7Schema,
    expected_thread_revision: RevisionSchema,
    target: CommandTarget,
    kind: z.enum(["instruction", "question", "notification", "reply"]),
    requirement: z.enum(["informational", "acknowledged", "answered"]),
    body: z
      .string()
      .min(1)
      .refine(
        (value) => value.trim().length > 0 && new TextEncoder().encode(value).byteLength <= 32_768,
      ),
    reply_to: UuidV7Schema.nullable(),
  })
  .strict()
  .superRefine((value, ctx) => {
    if (
      (value.kind === "reply") !== (value.reply_to !== null) ||
      ((value.kind === "reply" || value.kind === "notification") &&
        value.requirement !== "informational")
    )
      ctx.addIssue({ code: z.ZodIssueCode.custom, message: "Invalid reply or requirement" });
  });
const WaivePayload = z
  .object({
    message_id: UuidV7Schema,
    reason: z
      .string()
      .refine(
        (value) => value.trim().length > 0 && new TextEncoder().encode(value).byteLength <= 4096,
      ),
  })
  .strict();
const RetryPayload = z
  .object({
    run_id: UuidV7Schema,
    reason: z
      .string()
      .refine(
        (value) => value.trim().length > 0 && new TextEncoder().encode(value).byteLength <= 4096,
      ),
  })
  .strict();
export const InboxReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z
      .object({
        kind: z.enum(["employee_thread", "employee_message", "message_requirement_waiver", "run"]),
        id: UuidV7Schema,
      })
      .strict(),
  })
  .strict();
export type InboxReceipt = z.infer<typeof InboxReceiptSchema>;
export type InboxAttempt = Readonly<{
  action: InboxAction;
  body: string;
  key: string;
  projectId: string;
  expectedRevision: number;
  resourceId: string | null;
}>;
export function inboxAttempt(
  action: InboxAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): InboxAttempt {
  UuidV7Schema.parse(request.project_id);
  RevisionSchema.parse(request.expected_revision);
  IdempotencyKeySchema.parse(key);
  const payload =
    action === "open_employee_thread"
      ? OpenPayload.parse(request.payload)
      : action === "send_employee_message"
        ? SendPayload.parse(request.payload)
        : action === "waive_message_requirement"
          ? WaivePayload.parse(request.payload)
          : RetryPayload.parse(request.payload);
  return Object.freeze({
    action,
    body: JSON.stringify({
      project_id: request.project_id,
      expected_revision: request.expected_revision,
      payload,
    }),
    key,
    projectId: request.project_id,
    expectedRevision: request.expected_revision,
    resourceId:
      action === "waive_message_requirement"
        ? (payload as z.infer<typeof WaivePayload>).message_id
        : action === "retry_communication"
          ? (payload as z.infer<typeof RetryPayload>).run_id
          : null,
  });
}
