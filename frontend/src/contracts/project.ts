import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

// This control projection is not the complete Project configuration aggregate.
export const ProjectViewSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      revision: RevisionSchema,
      name: z.string(),
      execution_gate: z.enum(["open", "stopped"]),
    })
    .passthrough(),
);

export type ProjectView = z.infer<typeof ProjectViewSchema>;
