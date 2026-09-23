import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const endpoints = z.object({ blocker_task_id: UuidV7Schema, blocked_task_id: UuidV7Schema });
export const CreateDependencyRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema,
    payload: endpoints.extend({ required_condition: z.literal("task_done") }).strict(),
  })
  .strict();
export const RemoveDependencyRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema,
    payload: endpoints.strict(),
  })
  .strict();
export const DependencyCommandReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema),
    resource: z.object({ kind: z.literal("task_dependency"), id: UuidV7Schema }).strict(),
  })
  .strict();
export type DependencyCommandReceipt = z.infer<typeof DependencyCommandReceiptSchema>;
export type DependencyCommandAttempt = Readonly<{
  body: string;
  key: string;
  blockerId: string;
  blockedId: string;
  expectedRevision: number;
  action: "create_dependency" | "remove_dependency";
}>;

export function dependencyAttempt(
  action: DependencyCommandAttempt["action"],
  projectId: string,
  expectedRevision: number,
  blockerId: string,
  blockedId: string,
  key = crypto.randomUUID(),
): DependencyCommandAttempt {
  if (blockerId.toLowerCase() === blockedId.toLowerCase())
    throw new Error("A Task cannot depend on itself");
  const payload = { blocker_task_id: blockerId, blocked_task_id: blockedId };
  const request =
    action === "create_dependency"
      ? CreateDependencyRequestSchema.parse({
          project_id: projectId,
          expected_revision: expectedRevision,
          payload: { ...payload, required_condition: "task_done" },
        })
      : RemoveDependencyRequestSchema.parse({
          project_id: projectId,
          expected_revision: expectedRevision,
          payload,
        });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    blockerId,
    blockedId,
    expectedRevision,
    action,
  });
}
