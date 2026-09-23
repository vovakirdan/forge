import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { PipelineDefinitionInputSchema } from "./pipeline-management.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const PayloadSchema = PipelineDefinitionInputSchema.extend({
  name: z.string().refine((name) => name.trim().length > 0 && [...name].length <= 128),
}).strict();
const RequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: PayloadSchema,
  })
  .strict();
export type CreatePipelineAttempt = Readonly<{
  body: string;
  key: string;
  projectId: string;
  expectedProjectRevision: number;
  name: string;
}>;

export const InitialPipelineDefinition = {
  task_kinds: ["delivery"],
  entry_stage_id: "work",
  max_stage_visits: null,
  stages: [
    {
      id: "work",
      name: "Work",
      executor_kind: "employee",
      outcomes: ["done"],
      instructions: "Complete the assigned work and attach evidence.",
    },
  ],
  transitions: [
    { from_stage_id: "work", outcome: "done", target: { kind: "done" }, artifact_requirements: [] },
  ],
} as const;

export function createPipelineAttempt(
  projectId: string,
  expectedProjectRevision: number,
  name: string,
  definition: unknown,
  key: string = crypto.randomUUID(),
): CreatePipelineAttempt {
  const graph = PipelineDefinitionInputSchema.parse(definition);
  const request = RequestSchema.parse({
    project_id: projectId,
    expected_revision: expectedProjectRevision,
    payload: { name, ...graph },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    projectId: request.project_id,
    expectedProjectRevision: request.expected_revision,
    name: request.payload.name,
  });
}

export const CreatePipelineReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(2),
    resource: z.object({ kind: z.literal("pipeline"), id: UuidV7Schema }).strict(),
  })
  .strict();
