import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

export const GitRecoveryActionSchema = z.enum([
  "retry_git_integration",
  "accept_git_integration_result",
]);
export type GitRecoveryAction = z.infer<typeof GitRecoveryActionSchema>;

const ReasonSchema = z
  .string()
  .refine(
    (value) =>
      value.trim().length > 0 &&
      !value.includes("\0") &&
      new TextEncoder().encode(value).byteLength <= 4096,
  );
const RequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema,
    payload: z
      .object({
        operation_id: UuidV7Schema,
        expected_task_revision: RevisionSchema,
        reason: ReasonSchema,
      })
      .strict(),
  })
  .strict();

export const GitRecoveryReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("git_integration"), id: UuidV7Schema }).strict(),
  })
  .strict();

export type GitRecoveryAttempt = Readonly<{
  action: GitRecoveryAction;
  body: string;
  key: string;
  projectId: string;
  operationId: string;
  expectedRevision: number;
  expectedTaskRevision: number;
}>;

export function gitRecoveryAttempt(
  action: GitRecoveryAction,
  request: z.input<typeof RequestSchema>,
  key: string = crypto.randomUUID(),
): GitRecoveryAttempt {
  GitRecoveryActionSchema.parse(action);
  IdempotencyKeySchema.parse(key);
  const parsed = RequestSchema.parse(request);
  return Object.freeze({
    action,
    body: JSON.stringify(parsed),
    key,
    projectId: parsed.project_id,
    operationId: parsed.payload.operation_id,
    expectedRevision: parsed.expected_revision,
    expectedTaskRevision: parsed.payload.expected_task_revision,
  });
}
