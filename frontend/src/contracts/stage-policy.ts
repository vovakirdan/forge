import { z } from "zod";
import { StableKeySchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

// These are serialized read values: input defaults are already explicit in Core.
export const StageWorkspaceRequirementsSchema = preserveWireValue(
  z
    .object({
      kind: z.enum(["any", "filesystem", "git"]),
      access: z.enum(["read_only", "read_write"]),
    })
    .passthrough(),
);

export const StageAcceptancePolicySchema = preserveWireValue(
  z
    .object({
      kind: z.literal("candidate_review"),
      independent: z.boolean(),
      verdicts: z.record(StableKeySchema, z.enum(["accepted", "rejected", "inconclusive"])),
    })
    .passthrough(),
);

export const SystemStageActionSchema = preserveWireValue(
  z.discriminatedUnion("kind", [
    z
      .object({
        kind: z.literal("git_integration"),
        outcomes: z
          .object({
            applied: StableKeySchema,
            no_changes: StableKeySchema,
            stale_base: StableKeySchema,
          })
          .passthrough(),
        required_review_stages: z.array(StableKeySchema),
      })
      .passthrough(),
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
          .passthrough(),
      })
      .passthrough(),
  ]),
);

export type StageWorkspaceRequirements = z.infer<typeof StageWorkspaceRequirementsSchema>;
export type StageAcceptancePolicy = z.infer<typeof StageAcceptancePolicySchema>;
export type SystemStageAction = z.infer<typeof SystemStageActionSchema>;
