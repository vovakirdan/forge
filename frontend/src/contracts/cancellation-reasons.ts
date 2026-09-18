import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const CancellationReasonsViewSchema = preserveWireValue(
  z
    .object({
      project_id: UuidV7Schema,
      project_revision: RevisionSchema,
      reasons: z.array(
        z
          .object({
            id: StableKeySchema,
            display_name: z.string(),
            retired: z.boolean(),
          })
          .passthrough(),
      ),
    })
    .passthrough()
    .refine(
      (value) => new Set(value.reasons.map((reason) => reason.id)).size === value.reasons.length,
    ),
);
export type CancellationReasonsView = z.infer<typeof CancellationReasonsViewSchema>;
