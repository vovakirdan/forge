import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { TaskCommandReceiptSchema } from "./task-command.ts";
export { IdempotencyKeySchema } from "./task-command.ts";

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
export const DraftDefinitionOfDoneSchema = scalarString
  .refine((value) => !/^\p{White_Space}*$/u.test(value))
  .refine((value) => Array.from(value).length <= 20_000);
export const DraftPatchSchema = z
  .object({
    title: DraftTitleSchema.optional(),
    description: DraftDescriptionSchema.optional(),
    definition_of_done: DraftDefinitionOfDoneSchema.nullable().optional(),
  })
  .strict()
  .refine(
    (value) =>
      value.title !== undefined ||
      value.description !== undefined ||
      value.definition_of_done !== undefined,
  );
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
export const AmendDraftReceiptSchema = TaskCommandReceiptSchema;
export type AmendDraftRequest = z.infer<typeof AmendDraftRequestSchema>;
export type AmendDraftReceipt = z.infer<typeof AmendDraftReceiptSchema>;
