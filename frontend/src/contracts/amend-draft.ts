import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";

// Rust strings contain Unicode scalar values, not isolated UTF-16 surrogates.
const scalarString = z
  .string()
  .refine((value) => Array.from(value).every((character) => !/^[\uD800-\uDFFF]$/u.test(character)));
export const DraftTitleSchema = scalarString
  .refine((value) => !/^\p{White_Space}*$/u.test(value))
  .refine((value) => Array.from(value).length <= 240);
export const DraftDescriptionSchema = scalarString.refine(
  (value) => Array.from(value).length <= 50_000,
);
export const DraftPatchSchema = z
  .object({ title: DraftTitleSchema.optional(), description: DraftDescriptionSchema.optional() })
  .strict()
  .refine((value) => value.title !== undefined || value.description !== undefined);
export const AmendDraftRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        task_id: UuidV7Schema,
        expected_task_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
        patch: DraftPatchSchema,
      })
      .strict(),
  })
  .strict();
export const IdempotencyKeySchema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
export const AmendDraftReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("task"), id: UuidV7Schema }).strict(),
  })
  .strict();
export type AmendDraftRequest = z.infer<typeof AmendDraftRequestSchema>;
export type AmendDraftReceipt = z.infer<typeof AmendDraftReceiptSchema>;
