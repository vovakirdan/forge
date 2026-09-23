import { z } from "zod";
import { TimestampSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";
import { RunEvidenceReceiptSchema } from "./run-context-evidence.ts";

// Run details expose availability and receipt metadata only. Reject additive
// fields here so provider output, object keys and report bodies cannot be cached.
const AvailabilitySchema = z
  .object({ available: z.literal(true) })
  .strict()
  .nullable();
const IncidentSchema = z
  .object({
    id: UuidV7Schema,
    assessment: z.enum([
      "not_started_confirmed",
      "partial_work_observed",
      "external_effect_possible",
      "unknown",
    ]),
    created_at: TimestampSchema,
  })
  .strict();
const StreamSchema = z.object({ stream: z.string().min(1), incomplete: z.boolean() }).strict();

export const RunDiagnosticsSchema = preserveWireValue(
  z
    .object({
      runtime_report: AvailabilitySchema,
      handoff: AvailabilitySchema,
      incidents: z.array(IncidentSchema).max(100),
      evidence: z.array(RunEvidenceReceiptSchema).max(4_096),
      streams: z.array(StreamSchema),
      proxy_usage: AvailabilitySchema,
      git_source: AvailabilitySchema,
    })
    .strict(),
);

export type RunDiagnostics = z.infer<typeof RunDiagnosticsSchema>;
