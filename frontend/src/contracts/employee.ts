import { z } from "zod";
import { RevisionSchema, StableKeySchema, TimestampSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";
import { RunViewSchema } from "./run.ts";

const employeeSummaryShape = z.object({
  id: UuidV7Schema,
  name: z.string().min(1).max(200),
  role: z.string().min(1).max(128),
  state: z.enum(["enabled", "disabled", "retired"]),
  revision: RevisionSchema,
  max_concurrent_runs: z.number().int().min(1).max(65535),
});

export const EmployeeSummarySchema = preserveWireValue(employeeSummaryShape.passthrough());

const StageEligibilitySchema = preserveWireValue(
  z.discriminatedUnion("mode", [
    z.object({ mode: z.literal("any") }).passthrough(),
    z
      .object({
        mode: z.literal("only"),
        stages: z
          .array(
            z
              .object({ pipeline_version_id: UuidV7Schema, stage_id: StableKeySchema })
              .passthrough(),
          )
          .min(1),
      })
      .passthrough(),
  ]),
);

export const EmployeeProfileSchema = preserveWireValue(
  employeeSummaryShape
    .extend({
      project_id: UuidV7Schema,
      stage_eligibility: StageEligibilitySchema,
      created_at: TimestampSchema,
      updated_at: TimestampSchema,
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

export const EmployeeRunListResponseSchema = preserveWireValue(
  z
    .object({
      items: z.array(RunViewSchema),
      next_cursor: z.string().min(1).optional(),
    })
    .passthrough(),
);

export type EmployeeProfile = z.infer<typeof EmployeeProfileSchema>;
