import { z } from "zod";
import { RevisionSchema, TimestampSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

const slots = z.number().int().nonnegative().safe();

export const ProjectResourcesSchema = preserveWireValue(
  z
    .object({
      project_id: UuidV7Schema,
      policy: z
        .object({
          revision: RevisionSchema,
          host_max_runs: z.number().int().positive().safe(),
          project_max_runs: z.number().int().positive().safe(),
          credential_account_max_runs: z.number().int().positive().safe(),
          updated_at: TimestampSchema,
        })
        .passthrough(),
      occupancy: z
        .object({
          host_runs: slots,
          project_runs: slots,
          credential_account_runs: z.null(),
        })
        .passthrough(),
      observed_at: TimestampSchema,
    })
    .passthrough(),
);
