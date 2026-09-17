import { z } from "zod";
import { JsonObjectSchema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

// Technical receipts and reports are opaque JSON, not accepted Task artifacts
// or proof of success. Null means unavailable, not zero usage or an empty report.
export const RunDiagnosticsSchema = preserveWireValue(
  z
    .object({
      runtime_report: JsonObjectSchema.nullable(),
      handoff: JsonObjectSchema.nullable(),
      incidents: z.array(JsonObjectSchema),
      evidence: z.array(JsonObjectSchema),
      streams: z.array(
        z
          .object({
            stream: z.string(),
            incomplete: z.boolean(),
          })
          .passthrough(),
      ),
      proxy_usage: JsonObjectSchema.nullable(),
      git_source: JsonObjectSchema.nullable(),
    })
    .passthrough(),
);

export type RunDiagnostics = z.infer<typeof RunDiagnosticsSchema>;
