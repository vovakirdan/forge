import { z } from "zod";
import { IdempotencyKeySchema } from "./task-command.ts";
import { RevisionSchema, StableKeySchema, TaskKindSchema, UuidV7Schema } from "./common.ts";
import type { PipelineVersionView } from "./pipeline.ts";

const workspace = z
  .object({
    kind: z.enum(["any", "filesystem", "git"]),
    access: z.enum(["read_only", "read_write"]),
  })
  .strict();
const acceptance = z
  .object({
    kind: z.literal("candidate_review"),
    independent: z.boolean().optional(),
    verdicts: z.record(StableKeySchema, z.enum(["accepted", "rejected", "inconclusive"])),
  })
  .strict();
const systemAction = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("git_integration"),
      outcomes: z
        .object({
          applied: StableKeySchema,
          no_changes: StableKeySchema,
          stale_base: StableKeySchema,
        })
        .strict(),
      required_review_stages: z.array(StableKeySchema).max(64).optional(),
    })
    .strict(),
  z
    .object({
      kind: z.literal("project_hook"),
      hook_version_id: UuidV7Schema,
      outcomes: z
        .object({
          passed: StableKeySchema,
          failed: StableKeySchema,
          timed_out: StableKeySchema,
          skipped: StableKeySchema,
        })
        .strict(),
    })
    .strict(),
]);
const stage = z
  .object({
    id: StableKeySchema,
    name: z.string().min(1),
    executor_kind: z.enum(["employee", "human", "system", "external"]),
    outcomes: z.array(StableKeySchema).min(1).max(64),
    instructions: z.string().optional(),
    workspace: workspace.nullable().optional(),
    acceptance_policy: acceptance.nullable().optional(),
    system_action: systemAction.nullable().optional(),
  })
  .strict();
const target = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("stage"), stage_id: StableKeySchema }).strict(),
  z.object({ kind: z.literal("done") }).strict(),
  z.object({ kind: z.literal("cancelled") }).strict(),
]);
const transition = z
  .object({
    from_stage_id: StableKeySchema,
    outcome: StableKeySchema,
    target,
    artifact_requirements: z
      .array(
        z
          .object({
            kind: StableKeySchema,
            minimum_count: z.number().int().min(1).max(65535),
            scope: z.enum(["current_stage", "task_history"]).optional(),
          })
          .strict(),
      )
      .max(32)
      .optional(),
  })
  .strict();
export const PipelineDefinitionInputSchema = z
  .object({
    task_kinds: z.array(TaskKindSchema).min(1).max(2),
    entry_stage_id: StableKeySchema,
    max_stage_visits: z.number().int().min(1).max(1_000_000).nullable().optional(),
    stages: z.array(stage).min(1).max(128),
    transitions: z.array(transition).max(1024),
  })
  .strict();
export type PipelineDefinitionInput = z.infer<typeof PipelineDefinitionInputSchema>;

export const PipelineManagementActionSchema = z.enum([
  "publish_pipeline_version",
  "set_pipeline_default_version",
  "delete_pipeline",
]);
export type PipelineManagementAction = z.infer<typeof PipelineManagementActionSchema>;
export type PipelineManagementAttempt = Readonly<{
  action: PipelineManagementAction;
  body: string;
  key: string;
  pipelineId: string;
  expectedProjectRevision: number;
  resourceKind: "pipeline" | "pipeline_version";
}>;
const id = UuidV7Schema;
const revision = RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1);
const payloads = {
  publish_pipeline_version: z
    .object({
      pipeline_id: id,
      expected_pipeline_revision: revision,
      definition: PipelineDefinitionInputSchema,
      make_default: z.boolean().optional(),
    })
    .strict(),
  set_pipeline_default_version: z
    .object({ pipeline_id: id, expected_pipeline_revision: revision, pipeline_version_id: id })
    .strict(),
  delete_pipeline: z.object({ pipeline_id: id, expected_pipeline_revision: revision }).strict(),
};

export function pipelineManagementAttempt(
  action: PipelineManagementAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): PipelineManagementAttempt {
  const selected = PipelineManagementActionSchema.parse(action);
  const project = id.parse(request.project_id);
  const expected = revision.parse(request.expected_revision);
  const payload = payloads[selected].parse(request.payload);
  return Object.freeze({
    action: selected,
    body: JSON.stringify({ project_id: project, expected_revision: expected, payload }),
    key: IdempotencyKeySchema.parse(key),
    pipelineId: id.parse((payload as { pipeline_id: unknown }).pipeline_id),
    expectedProjectRevision: expected,
    resourceKind:
      selected === "publish_pipeline_version"
        ? ("pipeline_version" as const)
        : ("pipeline" as const),
  });
}

export function definitionFromVersion(version: PipelineVersionView): PipelineDefinitionInput {
  return PipelineDefinitionInputSchema.parse({
    task_kinds: version.task_kinds,
    entry_stage_id: version.entry_stage_id,
    max_stage_visits: version.max_stage_visits,
    stages: version.stages.map((item) => ({
      id: item.id,
      name: item.name,
      executor_kind: item.executor_kind,
      outcomes: item.outcomes,
      instructions: item.instructions,
      workspace: item.workspace,
      acceptance_policy: item.acceptance_policy,
      system_action: item.system_action,
    })),
    transitions: version.transitions.map((item) => ({
      from_stage_id: item.from_stage_id,
      outcome: item.outcome,
      target: item.target,
      artifact_requirements: item.artifact_requirements ?? [],
    })),
  });
}

export const PipelineManagementReceiptSchema = z
  .object({
    command_id: id,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(id).min(1),
    resource: z.object({ kind: z.enum(["pipeline", "pipeline_version"]), id }).strict(),
  })
  .strict();
