import { z } from "zod";
import { StableKeySchema, UuidV7Schema } from "./common.ts";
import { ExecutionAssignmentSchema } from "./execution-assignment.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";
import { RunDiagnosticsSchema } from "./run-diagnostics.ts";

export const RunDesiredStateSchema = z.enum([
  "provision_requested",
  "running",
  "stop_requested",
  "force_stop_requested",
  "stopped",
  "failed",
]);
export const RunObservedStateSchema = z.enum([
  "unknown",
  "provisioning",
  "running",
  "stopping",
  "stopped",
  "failed",
  "lost",
]);

const positiveSafeInteger = z.number().int().positive().safe();
// This read projection deliberately keeps desired and observed state apart.
// Parsing does not advance Task lifecycle or infer that an executor is stopped.
const runShape = z
  .object({
    id: UuidV7Schema,
    task_id: UuidV7Schema.nullable(),
    assignment: ExecutionAssignmentSchema,
    employee_id: UuidV7Schema.nullable(),
    stage_id: StableKeySchema.nullable(),
    attempt: z.number().int().positive().max(4_294_967_295),
    desired_state: RunDesiredStateSchema,
    observed_state: RunObservedStateSchema,
    lease_fencing_token: positiveSafeInteger,
    environment_epoch: positiveSafeInteger,
    last_observed_sequence: z.number().int().nonnegative().safe(),
    run_spec_version: z.number().int().positive().max(65_535),
  })
  .strict();

export const RunViewSchema = preserveWireValue(runShape);
export const RunDetailViewSchema = preserveWireValue(
  runShape.extend({ diagnostics: RunDiagnosticsSchema }).strict(),
);
export const RunListResponseSchema = preserveWireValue(
  z
    .object({
      items: z.array(RunViewSchema),
      next_cursor: z.string().optional(),
    })
    .strict(),
);

export type RunDesiredState = z.infer<typeof RunDesiredStateSchema>;
export type RunObservedState = z.infer<typeof RunObservedStateSchema>;
export type RunView = z.infer<typeof RunViewSchema>;
export type RunDetailView = z.infer<typeof RunDetailViewSchema>;
export type RunListResponse = z.infer<typeof RunListResponseSchema>;
