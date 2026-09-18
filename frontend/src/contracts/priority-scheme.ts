import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

// Match Rust's nonblank Unicode-scalar label, without trimming its wire value.
const displayName = z
  .string()
  .refine((value) => !/^\p{White_Space}*$/u.test(value))
  .refine((value) => Array.from(value).length <= 128)
  .refine((value) => Array.from(value).every((character) => !/^[\uD800-\uDFFF]$/u.test(character)));

export const PriorityLevelViewSchema = preserveWireValue(
  z
    .object({
      id: StableKeySchema,
      display_name: displayName,
      rank: z.number().int().min(-2_147_483_648).max(2_147_483_647),
      retired: z.boolean(),
    })
    .passthrough(),
);

export const PrioritySchemeViewSchema = preserveWireValue(
  z
    .object({
      project_id: UuidV7Schema,
      project_revision: RevisionSchema,
      default_level_id: StableKeySchema,
      levels: z.array(PriorityLevelViewSchema).min(1),
    })
    .passthrough()
    .refine((value) => new Set(value.levels.map((level) => level.id)).size === value.levels.length)
    .refine((value) =>
      value.levels.some((level) => level.id === value.default_level_id && !level.retired),
    ),
);

export type PriorityLevelView = z.infer<typeof PriorityLevelViewSchema>;
export type PrioritySchemeView = z.infer<typeof PrioritySchemeViewSchema>;
