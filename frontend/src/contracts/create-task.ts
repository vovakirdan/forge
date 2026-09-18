import { z } from "zod";
import { RevisionSchema, StableKeySchema, TaskKindSchema, UuidV7Schema } from "./common.ts";
import {
  DraftDescriptionSchema,
  DraftTitleSchema,
  DraftDefinitionOfDoneSchema,
} from "./amend-draft.ts";
export { DraftDefinitionOfDoneSchema } from "./amend-draft.ts";
export const CreateTaskRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        title: DraftTitleSchema,
        description: DraftDescriptionSchema,
        definition_of_done: DraftDefinitionOfDoneSchema.nullable(),
        kind: TaskKindSchema,
        priority: StableKeySchema,
        pipeline_version_id: UuidV7Schema,
        properties: z.object({}).strict(),
      })
      .strict(),
  })
  .strict();
export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>;
export type CreateTaskAttempt = Readonly<{ body: string; key: string }>;
