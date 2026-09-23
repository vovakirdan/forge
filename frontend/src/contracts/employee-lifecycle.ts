import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { EmployeeCommandReceiptSchema } from "./create-employee.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

export const EmployeeLifecycleActionSchema = z.enum([
  "enable_employee",
  "disable_employee",
  "retire_employee",
]);
export type EmployeeLifecycleAction = z.infer<typeof EmployeeLifecycleActionSchema>;

export const EmployeeLifecycleRequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.min(1).max(Number.MAX_SAFE_INTEGER - 1),
    payload: z
      .object({
        employee_id: UuidV7Schema,
        expected_employee_revision: RevisionSchema.min(1).max(Number.MAX_SAFE_INTEGER - 1),
        reason: z
          .string()
          .trim()
          .min(1)
          .refine((reason) => !reason.includes("\0") && [...reason].length <= 10_000)
          .optional(),
      })
      .strict(),
  })
  .strict();

export { EmployeeCommandReceiptSchema };
export type EmployeeLifecycleRequest = z.infer<typeof EmployeeLifecycleRequestSchema>;
export type EmployeeLifecycleAttempt = Readonly<{
  action: EmployeeLifecycleAction;
  body: string;
  key: string;
}>;

export function employeeLifecycleAttempt(
  action: EmployeeLifecycleAction,
  request: EmployeeLifecycleRequest,
  key = crypto.randomUUID(),
): EmployeeLifecycleAttempt {
  return Object.freeze({
    action: EmployeeLifecycleActionSchema.parse(action),
    body: JSON.stringify(EmployeeLifecycleRequestSchema.parse(request)),
    key: IdempotencyKeySchema.parse(key),
  });
}
