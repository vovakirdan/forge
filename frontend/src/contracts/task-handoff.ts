import { z } from "zod";
import { TimestampSchema, UuidV7Schema } from "./common.ts";

export const TaskHandoffSchema = z
  .object({
    id: UuidV7Schema,
    project_id: UuidV7Schema,
    task_id: UuidV7Schema,
    kind: z.enum(["accepted", "interrupted"]),
    source_kind: z.enum(["run", "hook", "git_integration", "command"]),
    source_id: UuidV7Schema,
    created_at: TimestampSchema,
    content_availability: z.literal("unavailable"),
  })
  .strict();

export const TaskHandoffPageSchema = z
  .object({
    items: z.array(TaskHandoffSchema).max(20),
    next_cursor: UuidV7Schema.optional(),
  })
  .strict();
