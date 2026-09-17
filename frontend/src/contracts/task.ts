import { z } from "zod";
import {
  ActorReferenceSchema,
  JsonObjectSchema,
  JsonValueSchema,
  LifecycleStatusSchema,
  RevisionSchema,
  StableKeySchema,
  TaskKindSchema,
  TimestampSchema,
  UuidV7Schema,
} from "./common.ts";
import { TaskPropertiesSchema } from "./properties.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const ArtifactViewSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      kind: StableKeySchema,
      title: z.string(),
      metadata: JsonObjectSchema,
      // Inline JSON may resemble an object reference; do not interpret or fetch it.
      body: JsonValueSchema,
      created_at: TimestampSchema,
    })
    .passthrough(),
);

export const WaitConditionViewSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      kind: StableKeySchema,
      detail: z.string().nullable(),
      source_stage_id: StableKeySchema.nullable(),
      created_by: ActorReferenceSchema,
      created_at: TimestampSchema,
    })
    .passthrough(),
);

const taskSummaryShape = z
  .object({
    id: UuidV7Schema,
    // Core currently emits TASK-001; OpenAPI's non-zero first digit is stale.
    key: z.string().min(1),
    revision: RevisionSchema,
    title: z.string(),
    kind: TaskKindSchema,
    lifecycle: LifecycleStatusSchema,
    current_stage_id: StableKeySchema.nullable(),
    priority: StableKeySchema,
    pipeline_version_id: UuidV7Schema,
    updated_at: TimestampSchema,
  })
  .passthrough();

export const TaskSummaryViewSchema = preserveWireValue(taskSummaryShape);
export const TaskDetailViewSchema = preserveWireValue(
  taskSummaryShape.extend({
    description: z.string(),
    definition_of_done: z.string().nullable(),
    properties: TaskPropertiesSchema,
    artifacts: z.array(ArtifactViewSchema),
    wait_conditions: z.array(WaitConditionViewSchema),
  }),
);

export const TaskListResponseSchema = preserveWireValue(
  z
    .object({
      items: z.array(TaskSummaryViewSchema),
      // Core omits the cursor on the final page; it does not serialize null.
      next_cursor: z.string().optional(),
    })
    .passthrough(),
);

export type ArtifactView = z.infer<typeof ArtifactViewSchema>;
export type WaitConditionView = z.infer<typeof WaitConditionViewSchema>;
export type TaskSummaryView = z.infer<typeof TaskSummaryViewSchema>;
export type TaskDetailView = z.infer<typeof TaskDetailViewSchema>;
export type TaskListResponse = z.infer<typeof TaskListResponseSchema>;
