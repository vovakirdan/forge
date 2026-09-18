import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";

export const ApproveTaskRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        task_id: UuidV7Schema,
        expected_task_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
      })
      .strict(),
  })
  .strict();
export type ApproveTaskRequest = z.infer<typeof ApproveTaskRequestSchema>;
