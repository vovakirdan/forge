import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";
export const ManagementActionSchema = z.enum([
  "start_project_execution",
  "stop_project_execution",
  "pause_task",
  "resume_task",
  "stop_employee",
  "submit_human_resolution",
  "reroute_escalation",
  "accept_run_recovery_assessment",
]);
export type ManagementAction = z.infer<typeof ManagementActionSchema>;
const Reason = z
  .string()
  .refine(
    (value) =>
      value.trim().length > 0 &&
      !value.includes("\0") &&
      new TextEncoder().encode(value).byteLength <= 10_000,
  );
const ProjectPayload = z.object({ reason: Reason.nullable() }).strict();
const StopMode = z.enum(["graceful", "force"]);
const PausePayload = z
  .object({
    task_id: UuidV7Schema,
    expected_task_revision: RevisionSchema,
    mode: StopMode,
    reason: Reason.nullable(),
  })
  .strict();
const StopEmployeePayload = z
  .object({
    employee_id: UuidV7Schema,
    expected_employee_revision: RevisionSchema,
    mode: StopMode,
    reason: Reason.nullable(),
  })
  .strict();
const ResumePayload = z
  .object({
    task_id: UuidV7Schema,
    expected_task_revision: RevisionSchema,
    wait_condition_id: UuidV7Schema,
  })
  .strict();
const SubmitResolutionPayload = z
  .object({
    escalation_id: UuidV7Schema,
    expected_escalation_revision: RevisionSchema,
    assignment_id: UuidV7Schema,
    lease_generation: RevisionSchema,
    answer: z
      .object({
        disposition: z.enum(["continue_stage", "needs_management_change", "forward_to_human"]),
        summary: z
          .string()
          .refine(
            (value) =>
              value.trim().length > 0 &&
              !value.includes("\0") &&
              new TextEncoder().encode(value).byteLength <= 20_000,
          ),
        recommended_outcome_key: StableKeySchema.nullable(),
      })
      .strict(),
  })
  .strict();
const ReroutePayload = z
  .object({
    escalation_id: UuidV7Schema,
    expected_escalation_revision: RevisionSchema,
    reason: Reason,
  })
  .strict();
const RecoveryAssessmentPayload = z
  .object({
    run_id: UuidV7Schema,
    assessment: z.literal("not_started_confirmed"),
  })
  .strict();
export const ManagementReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z
      .object({
        kind: z.enum(["project", "task", "employee", "escalation", "run"]),
        id: UuidV7Schema,
      })
      .strict(),
  })
  .strict();
export type ManagementReceipt = z.infer<typeof ManagementReceiptSchema>;
export type ManagementAttempt = Readonly<{
  action: ManagementAction;
  body: string;
  key: string;
  projectId: string;
  expectedRevision: number;
  resourceId: string;
}>;
export function managementAttempt(
  action: ManagementAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): ManagementAttempt {
  const projectId = UuidV7Schema.parse(request.project_id);
  const expectedRevision = RevisionSchema.parse(request.expected_revision);
  IdempotencyKeySchema.parse(key);
  const payload =
    action === "start_project_execution" || action === "stop_project_execution"
      ? ProjectPayload.parse(request.payload)
      : action === "pause_task"
        ? PausePayload.parse(request.payload)
        : action === "resume_task"
          ? ResumePayload.parse(request.payload)
          : action === "submit_human_resolution"
            ? SubmitResolutionPayload.parse(request.payload)
            : action === "reroute_escalation"
              ? ReroutePayload.parse(request.payload)
              : action === "accept_run_recovery_assessment"
                ? RecoveryAssessmentPayload.parse(request.payload)
                : StopEmployeePayload.parse(request.payload);
  const resourceId =
    action === "pause_task" || action === "resume_task"
      ? (payload as z.infer<typeof PausePayload> | z.infer<typeof ResumePayload>).task_id
      : action === "stop_employee"
        ? (payload as z.infer<typeof StopEmployeePayload>).employee_id
        : action === "submit_human_resolution" || action === "reroute_escalation"
          ? (payload as z.infer<typeof SubmitResolutionPayload> | z.infer<typeof ReroutePayload>)
              .escalation_id
          : action === "accept_run_recovery_assessment"
            ? (payload as z.infer<typeof RecoveryAssessmentPayload>).run_id
            : projectId;
  return Object.freeze({
    action,
    body: JSON.stringify({ project_id: projectId, expected_revision: expectedRevision, payload }),
    key,
    projectId,
    expectedRevision,
    resourceId,
  });
}
