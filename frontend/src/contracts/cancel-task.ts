import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";

export const CancellationNoteSchema = z
  .string()
  .refine((value) => !/^\p{White_Space}*$/u.test(value))
  .refine((value) => Array.from(value).length <= 20_000)
  .refine((value) => Array.from(value).every((character) => !/^[\uD800-\uDFFF]$/u.test(character)));
export const CancelTaskRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        task_id: UuidV7Schema,
        expected_task_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
        cancellation_reason_key: StableKeySchema,
        note: CancellationNoteSchema.nullable().optional(),
      })
      .strict(),
  })
  .strict();
export type CancelTaskRequest = z.infer<typeof CancelTaskRequestSchema>;
