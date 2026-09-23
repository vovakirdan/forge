import { z } from "zod";
import { RevisionSchema, TaskKindSchema, TimestampSchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const positive = z.number().int().positive().safe();
const base = z
  .object({
    name: z.string().refine((value) => value.trim().length > 0 && value.length <= 128),
    image: z.string().regex(/^.+@sha256:[0-9a-fA-F]{64}$/),
    command: z.array(z.string().max(32768)).min(1).max(128),
    workdir: z.string().min(1).max(4096),
    limits: z
      .object({
        cpu_millis: positive,
        memory_bytes: positive.min(16 * 1024 * 1024),
        pids: positive,
        wall_seconds: positive,
        stop_grace_seconds: positive,
      })
      .strict()
      .refine((limits) => limits.stop_grace_seconds <= limits.wall_seconds),
    max_output_bytes: positive.max(256 * 1024 * 1024),
    applicable_task_kinds: z.array(TaskKindSchema).max(2).default([]),
    required: z.boolean(),
  })
  .strict();
const validWorkdir = (value: { workdir: string }) =>
  value.workdir === "." ||
  value.workdir.split("/").every((part) => part && part !== "." && part !== "..");
export const ProjectHookInputSchema = base.refine(validWorkdir);

export const ProjectHookVersionSchema = base
  .extend({
    id: UuidV7Schema,
    project_id: UuidV7Schema,
    created_at: TimestampSchema,
  })
  .strict()
  .refine(validWorkdir);
export const ProjectHookListSchema = z
  .object({ items: z.array(ProjectHookVersionSchema), next_cursor: UuidV7Schema.optional() })
  .strict();
export type ProjectHookVersion = z.infer<typeof ProjectHookVersionSchema>;

export const ProjectHookInvocationSchema = z
  .object({
    id: UuidV7Schema,
    task_id: UuidV7Schema,
    pipeline_version_id: UuidV7Schema,
    stage_id: z.string().min(1).max(128),
    stage_visit: RevisionSchema,
    hook_version_id: UuidV7Schema,
    candidate_proposal_id: UuidV7Schema.nullable(),
    run_id: UuidV7Schema.nullable(),
    state: z.enum(["running", "completed", "held"]),
    verdict: z.enum(["passed", "failed", "timed_out", "skipped"]).nullable(),
    mapped_outcome: z.string().min(1).max(128).nullable(),
    artifact_id: UuidV7Schema.nullable(),
    created_at: TimestampSchema,
    updated_at: TimestampSchema,
  })
  .strict()
  .refine(
    (row) =>
      (row.state === "completed" && row.verdict !== null && row.mapped_outcome !== null) ||
      (row.state !== "completed" && row.verdict === null && row.mapped_outcome === null),
    "Only completed hook results map to a Pipeline outcome",
  );
export const ProjectHookInvocationListSchema = z
  .object({
    items: z.array(ProjectHookInvocationSchema).max(100),
    next_cursor: UuidV7Schema.optional(),
  })
  .strict();
export type ProjectHookInvocationList = z.infer<typeof ProjectHookInvocationListSchema>;

const RequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: ProjectHookInputSchema,
  })
  .strict();
export type ProjectHookAttempt = Readonly<{
  body: string;
  key: string;
  projectId: string;
  expectedProjectRevision: number;
}>;
export function configureProjectHookAttempt(
  projectId: string,
  expectedProjectRevision: number,
  input: unknown,
  key: string = crypto.randomUUID(),
): ProjectHookAttempt {
  const request = RequestSchema.parse({
    project_id: projectId,
    expected_revision: expectedProjectRevision,
    payload: input,
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    projectId: request.project_id,
    expectedProjectRevision: request.expected_revision,
  });
}
export const ProjectHookReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("project_hook_version"), id: UuidV7Schema }).strict(),
  })
  .strict();
