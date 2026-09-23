import { z } from "zod";
import { RevisionSchema, StableKeySchema, TimestampSchema, UuidV7Schema } from "./common.ts";

const Commit = z.string().regex(/^(?!0+$)(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{64})$/);
const Candidate = z.object({ commit: Commit, tree: Commit });
export const GitSourceSettingSchema = z.object({
  revision: RevisionSchema,
  policy: z.discriminatedUnion("mode", [
    z.object({ mode: z.literal("latest_target") }),
    z.object({ mode: z.literal("pinned_commit"), commit: Commit }),
  ]),
});

// Core's browser-facing manifest omits storage coordinates. Zod also strips unknown keys
// so an additive field cannot become a URL or visible object key through this view.
export const SafeFileManifestSchema = z.object({
  schema_version: z.literal(1),
  project_id: UuidV7Schema,
  source_task_id: UuidV7Schema,
  files: z
    .array(
      z.object({
        path: z.string().min(1).max(1024),
        size_bytes: z.number().int().nonnegative().max(16_777_216),
        executable: z.boolean(),
        sha256: z.string().regex(/^[0-9a-f]{64}$/),
      }),
    )
    .min(1)
    .max(64),
});
export const FileSnapshotSchema = z
  .object({
    id: UuidV7Schema,
    artifact_id: UuidV7Schema,
    task_id: UuidV7Schema,
    title: z.string(),
    state: z.enum(["pending", "sealed", "failed"]),
    error_code: z.string().nullable(),
    manifest: SafeFileManifestSchema.nullable(),
    created_at: TimestampSchema,
  })
  .superRefine((value, ctx) => {
    if ((value.state === "sealed") !== (value.manifest !== null))
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Snapshot state and manifest disagree",
      });
  });
const Page = <T extends z.ZodTypeAny>(item: T) =>
  z.object({
    items: z.array(item).max(20),
    next_cursor: UuidV7Schema.optional(),
  });
export const FileSnapshotsSchema = Page(FileSnapshotSchema);
export const FileInputsSchema = z.object({
  items: z.array(z.object({ artifact_id: UuidV7Schema, manifest: SafeFileManifestSchema })).max(8),
});
export const CandidateReviewsSchema = Page(
  z.object({
    id: UuidV7Schema,
    project_id: UuidV7Schema,
    task_id: UuidV7Schema,
    pipeline_version_id: UuidV7Schema,
    stage_id: StableKeySchema,
    stage_visit: RevisionSchema,
    candidate_proposal_id: UuidV7Schema,
    candidate: Candidate,
    subject_artifact_ids: z.array(UuidV7Schema).max(64),
    verdict_artifact_ids: z.array(UuidV7Schema).max(64),
    outcome: StableKeySchema,
    verdict: z.enum(["accepted", "rejected", "inconclusive"]),
    recorded_at: TimestampSchema,
  }),
);
export const GitIntegrationsSchema = Page(
  z.object({
    id: UuidV7Schema,
    task_id: UuidV7Schema,
    stage_id: StableKeySchema,
    stage_visit: RevisionSchema,
    candidate_proposal_id: UuidV7Schema,
    candidate: Candidate,
    state: z.enum(["preparing", "prepared", "applying", "held", "completed", "retired"]),
    prepared_merge: z
      .object({ expected_target: Commit.nullable(), merge_commit: Commit })
      .nullable(),
    result_code: z
      .enum([
        "prepared",
        "applied",
        "no_changes",
        "stale_base",
        "retryable",
        "no_preparation",
        "unknown",
        "busy",
        "refused",
        "unavailable",
      ])
      .nullable(),
    created_at: TimestampSchema,
  }),
);
export type FileSnapshots = z.infer<typeof FileSnapshotsSchema>;
export type FileInputs = z.infer<typeof FileInputsSchema>;
export type CandidateReviews = z.infer<typeof CandidateReviewsSchema>;
export type GitIntegrations = z.infer<typeof GitIntegrationsSchema>;
