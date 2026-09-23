import { z } from "zod";
import { preserveWireValue } from "./preserve-wire-value.ts";
import {
  RevisionSchema,
  StableKeySchema,
  TaskKindSchema,
  TimestampSchema,
  UuidV7Schema,
} from "./common.ts";
import {
  StageAcceptancePolicySchema,
  StageWorkspaceRequirementsSchema,
  SystemStageActionSchema,
} from "./stage-policy.ts";

export const PipelineStageViewSchema = preserveWireValue(
  z
    .object({
      id: StableKeySchema,
      name: z.string(),
      executor_kind: z.enum(["employee", "human", "system", "external"]),
      outcomes: z.array(StableKeySchema),
      instructions: z.string(),
      workspace: StageWorkspaceRequirementsSchema.nullable(),
      acceptance_policy: StageAcceptancePolicySchema.nullable(),
      system_action: SystemStageActionSchema.nullable(),
    })
    .passthrough(),
);

// HTTP uses a tagged target, unlike the internal domain enum serialization.
export const PipelineTargetViewSchema = preserveWireValue(
  z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("stage"), stage_id: StableKeySchema }).passthrough(),
    z.object({ kind: z.literal("done") }).passthrough(),
    z.object({ kind: z.literal("cancelled") }).passthrough(),
  ]),
);

export const ArtifactRequirementViewSchema = preserveWireValue(
  z
    .object({
      kind: StableKeySchema,
      minimum_count: z.number().int().positive().max(65_535),
      scope: z.enum(["current_stage", "task_history"]),
    })
    .passthrough(),
);

export const PipelineTransitionViewSchema = preserveWireValue(
  z
    .object({
      from_stage_id: StableKeySchema,
      outcome: StableKeySchema,
      target: PipelineTargetViewSchema,
      artifact_requirements: z.array(ArtifactRequirementViewSchema).optional(),
    })
    .passthrough(),
);

export const PipelineVersionViewSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      pipeline_id: UuidV7Schema,
      version: z.number().int().positive().max(4_294_967_295),
      name: z.string(),
      catalog_revision: RevisionSchema,
      default_version_id: UuidV7Schema,
      latest_version: z.number().int().positive().max(4_294_967_295),
      deleted_at: TimestampSchema.nullable(),
      task_kinds: z.array(TaskKindSchema),
      entry_stage_id: StableKeySchema,
      max_stage_visits: z.number().int().positive().max(4_294_967_295).nullable(),
      stages: z.array(PipelineStageViewSchema),
      transitions: z.array(PipelineTransitionViewSchema),
    })
    .passthrough(),
);

export const PipelineVersionListResponseSchema = preserveWireValue(
  z
    .object({
      items: z.array(PipelineVersionViewSchema),
      next_cursor: z.string().optional(),
    })
    .passthrough(),
);

export type PipelineStageView = z.infer<typeof PipelineStageViewSchema>;
export type PipelineTargetView = z.infer<typeof PipelineTargetViewSchema>;
export type ArtifactRequirementView = z.infer<typeof ArtifactRequirementViewSchema>;
export type PipelineTransitionView = z.infer<typeof PipelineTransitionViewSchema>;
export type PipelineVersionView = z.infer<typeof PipelineVersionViewSchema>;
export type PipelineVersionListResponse = z.infer<typeof PipelineVersionListResponseSchema>;

export const PipelineCatalogItemSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      name: z.string().min(1),
      revision: RevisionSchema,
      default_version_id: UuidV7Schema.nullable(),
      latest_version_id: UuidV7Schema.nullable(),
      latest_version: z.number().int().positive().max(4_294_967_295).nullable(),
      deleted_at: TimestampSchema.nullable(),
      pinned_task_count: z.number().int().nonnegative().safe(),
    })
    .passthrough(),
);

export const PipelineCatalogListSchema = preserveWireValue(
  z
    .object({
      items: z.array(PipelineCatalogItemSchema).max(20),
      next_cursor: UuidV7Schema.optional(),
    })
    .passthrough(),
);

export type PipelineCatalogItem = z.infer<typeof PipelineCatalogItemSchema>;
