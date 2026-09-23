import { z } from "zod";
import { RevisionSchema, TimestampSchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const Reason = z
  .string()
  .refine(
    (value) =>
      value.trim().length > 0 &&
      !value.includes("\0") &&
      new TextEncoder().encode(value).byteLength <= 10_000,
  );
const payloads = {
  set_next_run_employee: z
    .object({
      task_id: UuidV7Schema,
      expected_task_revision: RevisionSchema,
      employee_id: UuidV7Schema,
      reason: Reason.nullable(),
    })
    .strict(),
  clear_next_run_employee: z
    .object({
      task_id: UuidV7Schema,
      expected_task_revision: RevisionSchema,
      reason: Reason.nullable(),
    })
    .strict(),
  schedule_task_resume: z
    .object({
      task_id: UuidV7Schema,
      expected_task_revision: RevisionSchema,
      wait_condition_id: UuidV7Schema,
      not_before: TimestampSchema,
      reason: Reason,
    })
    .strict(),
  cancel_task_resume: z.object({ schedule_id: UuidV7Schema, reason: Reason.nullable() }).strict(),
};
export const ManagerPlanningActionSchema = z.enum([
  "set_next_run_employee",
  "clear_next_run_employee",
  "schedule_task_resume",
  "cancel_task_resume",
]);
export type ManagerPlanningAction = z.infer<typeof ManagerPlanningActionSchema>;
export type ManagerPlanningAttempt = Readonly<{
  action: ManagerPlanningAction;
  body: string;
  key: string;
  projectId: string;
  expectedRevision: number;
  resourceId: string | null;
}>;
export function managerPlanningAttempt(
  action: ManagerPlanningAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): ManagerPlanningAttempt {
  const projectId = UuidV7Schema.parse(request.project_id);
  const expectedRevision = RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1).parse(
    request.expected_revision,
  );
  const payload = payloads[action].parse(request.payload) as Record<string, unknown>;
  const resourceId =
    action === "cancel_task_resume"
      ? (payload["schedule_id"] as string)
      : action === "schedule_task_resume"
        ? null
        : (payload["task_id"] as string);
  return Object.freeze({
    action,
    body: JSON.stringify({ project_id: projectId, expected_revision: expectedRevision, payload }),
    key: IdempotencyKeySchema.parse(key),
    projectId,
    expectedRevision,
    resourceId,
  });
}
export const ManagerPlanningReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z
      .object({ kind: z.enum(["task", "task_resume_schedule"]), id: UuidV7Schema })
      .strict(),
  })
  .strict();
