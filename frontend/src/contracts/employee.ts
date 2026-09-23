import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const EmployeeSummarySchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      name: z.string().min(1).max(200),
      role: z.string().min(1).max(128),
      state: z.enum(["enabled", "disabled", "retired"]),
      revision: RevisionSchema,
      max_concurrent_runs: z.number().int().min(1).max(65535),
    })
    .passthrough(),
);

export const EmployeeListResponseSchema = preserveWireValue(
  z
    .object({
      items: z.array(EmployeeSummarySchema),
      next_cursor: z.string().min(1).optional(),
    })
    .passthrough(),
);
