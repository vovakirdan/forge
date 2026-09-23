import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { EmployeeEligibilityInputSchema, EmployeeCommandReceiptSchema } from "./create-employee.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

export const AmendEmployeeRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.min(1).max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        employee_id: UuidV7Schema,
        expected_employee_revision: RevisionSchema.min(1).max(Number.MAX_SAFE_INTEGER - 1),
        patch: z
          .object({
            name: z.string().trim().min(1).max(200).optional(),
            role: z.string().trim().min(1).max(128).optional(),
            max_concurrent_runs: z.number().int().min(1).max(65535).optional(),
            stage_eligibility: EmployeeEligibilityInputSchema.optional(),
          })
          .strict()
          .refine((patch) => Object.keys(patch).length > 0, "Choose at least one change"),
      })
      .strict(),
  })
  .strict();

export { EmployeeCommandReceiptSchema };
export type AmendEmployeeRequest = z.infer<typeof AmendEmployeeRequestSchema>;
export type AmendEmployeeAttempt = Readonly<{ body: string; key: string }>;

export function amendEmployeeAttempt(
  request: AmendEmployeeRequest,
  key = crypto.randomUUID(),
): AmendEmployeeAttempt {
  return Object.freeze({
    body: JSON.stringify(AmendEmployeeRequestSchema.parse(request)),
    key: IdempotencyKeySchema.parse(key),
  });
}
