import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";

export const SetTaskPriorityRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        task_id: UuidV7Schema,
        expected_task_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
        priority: StableKeySchema,
      })
      .strict(),
  })
  .strict();
export type SetTaskPriorityRequest = z.infer<typeof SetTaskPriorityRequestSchema>;
