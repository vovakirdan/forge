import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const StageSchema = z
  .object({ pipeline_version_id: UuidV7Schema, stage_id: StableKeySchema })
  .strict();
export const EmployeeEligibilityInputSchema = z
  .discriminatedUnion("mode", [
    z.object({ mode: z.literal("any") }).strict(),
    z.object({ mode: z.literal("only"), stages: z.array(StageSchema).min(1).max(128) }).strict(),
  ])
  .superRefine((value, context) => {
    if (
      value.mode === "only" &&
      new Set(value.stages.map((stage) => `${stage.pipeline_version_id}:${stage.stage_id}`))
        .size !== value.stages.length
    )
      context.addIssue({ code: z.ZodIssueCode.custom, message: "Duplicate stage" });
  });

export const CreateEmployeeRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        name: z.string().trim().min(1).max(200),
        role: z.string().trim().min(1).max(128),
        stage_eligibility: EmployeeEligibilityInputSchema,
      })
      .strict(),
  })
  .strict();

export const EmployeeCommandReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("employee"), id: UuidV7Schema }).strict(),
  })
  .strict();

export type CreateEmployeeRequest = z.infer<typeof CreateEmployeeRequestSchema>;
export type CreateEmployeeAttempt = Readonly<{ body: string; key: string }>;
export type EmployeeCommandReceipt = z.infer<typeof EmployeeCommandReceiptSchema>;

export function createEmployeeAttempt(
  request: CreateEmployeeRequest,
  key = crypto.randomUUID(),
): CreateEmployeeAttempt {
  return Object.freeze({
    body: JSON.stringify(CreateEmployeeRequestSchema.parse(request)),
    key: IdempotencyKeySchema.parse(key),
  });
}
