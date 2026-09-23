import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { SystemJobBindingInputSchema } from "./system-job-command.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const RequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z.object({ employee_id: UuidV7Schema, binding: SystemJobBindingInputSchema }).strict(),
  })
  .strict();
export type EmployeeRuntimeAttempt = Readonly<{
  body: string;
  key: string;
  projectId: string;
  employeeId: string;
  expectedProjectRevision: number;
}>;
export function configureEmployeeRuntimeAttempt(
  projectId: string,
  employeeId: string,
  expectedProjectRevision: number,
  binding: unknown,
  key: string = crypto.randomUUID(),
): EmployeeRuntimeAttempt {
  const request = RequestSchema.parse({
    project_id: projectId,
    expected_revision: expectedProjectRevision,
    payload: { employee_id: employeeId, binding },
  });
  if (
    request.payload.binding.execution_profile.project_id.toLowerCase() !==
      projectId.toLowerCase() ||
    request.payload.binding.execution_profile.credential_binding.project_id.toLowerCase() !==
      projectId.toLowerCase()
  )
    throw new Error("Runtime profile belongs to another Project");
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    projectId: request.project_id,
    employeeId: request.payload.employee_id,
    expectedProjectRevision: request.expected_revision,
  });
}
export const EmployeeRuntimeReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("employee"), id: UuidV7Schema }).strict(),
  })
  .strict();
