import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";

export const IdempotencyKeySchema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
export const TaskCommandReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("task"), id: UuidV7Schema }).strict(),
  })
  .strict();
export type TaskCommandReceipt = z.infer<typeof TaskCommandReceiptSchema>;
export type TaskCommandAttempt = Readonly<{ body: string; key: string; taskId: string }>;
