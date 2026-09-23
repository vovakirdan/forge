import { z } from "zod";
import { RevisionSchema, StableKeySchema, TimestampSchema, UuidV7Schema } from "./common.ts";

const CoordinatesSchema = z
  .object({
    context_snapshot_id: UuidV7Schema,
    run_id: UuidV7Schema,
    project_id: UuidV7Schema,
    task_id: UuidV7Schema,
    employee_id: UuidV7Schema,
    pipeline_version_id: UuidV7Schema,
    stage_id: StableKeySchema,
    stage_visit: RevisionSchema.nullable(),
    task_revision_before_dispatch: RevisionSchema,
    run_spec_id: UuidV7Schema,
  })
  .strict();

export const RunContextCoordinatesSchema = z.discriminatedUnion("availability", [
  z.object({ availability: z.literal("available"), coordinates: CoordinatesSchema }).strict(),
  z
    .object({ availability: z.literal("unavailable"), run_id: UuidV7Schema, coordinates: z.null() })
    .strict(),
]);

export const RunEvidenceReceiptSchema = z
  .object({
    id: UuidV7Schema,
    run_id: UuidV7Schema,
    task_id: UuidV7Schema.nullable(),
    stream: z.enum(["stdout", "stderr", "diagnostic"]),
    sequence: RevisionSchema,
    sha256: z.string().regex(/^[0-9a-f]{64}$/),
    size_bytes: z.number().int().safe().positive(),
    redaction_policy_reference: z.string().min(1),
    storage_state: z.enum(["pending_upload", "stored", "unconfirmed"]),
    created_at: TimestampSchema,
    content_availability: z.literal("unavailable"),
  })
  .strict();

export const RunEvidencePageSchema = z
  .object({
    items: z.array(RunEvidenceReceiptSchema).max(50),
    next_cursor: UuidV7Schema.optional(),
  })
  .strict();

export type RunContextCoordinates = z.infer<typeof RunContextCoordinatesSchema>;
export type RunEvidencePage = z.infer<typeof RunEvidencePageSchema>;
