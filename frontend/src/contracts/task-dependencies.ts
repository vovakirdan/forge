import { z } from "zod";
import { LifecycleStatusSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const DependencyDirectionSchema = z.enum(["blocked_by", "blocks"]);
export type DependencyDirection = z.infer<typeof DependencyDirectionSchema>;

export const TaskDependencyViewSchema = preserveWireValue(
  z
    .object({
      blocker_task_id: UuidV7Schema,
      blocked_task_id: UuidV7Schema,
      required_condition: z.literal("task_done"),
      related_task: z
        .object({
          id: UuidV7Schema,
          key: z.string().min(1),
          title: z.string(),
          lifecycle: LifecycleStatusSchema,
        })
        .passthrough(),
      condition_state: z.enum(["pending", "satisfied", "blocker_cancelled"]),
    })
    .passthrough(),
);
export const TaskDependencyListResponseSchema = preserveWireValue(
  z
    .object({
      items: z.array(TaskDependencyViewSchema),
      next_cursor: z.string().optional(),
    })
    .passthrough(),
);
export type TaskDependencyView = z.infer<typeof TaskDependencyViewSchema>;
