import { z } from "zod";
import { RevisionSchema, StableKeySchema, TimestampSchema, UuidV7Schema } from "./common.ts";
const Page = <T extends z.ZodTypeAny>(item: T) =>
  z.object({ items: z.array(item).max(20), next_cursor: UuidV7Schema.optional() });
export const ResumeScheduleSchema = z.object({
  id: UuidV7Schema,
  task_id: UuidV7Schema,
  expected_task_revision: RevisionSchema,
  pipeline_version_id: UuidV7Schema,
  stage_id: StableKeySchema,
  stage_visit: RevisionSchema,
  wait_condition_id: UuidV7Schema,
  not_before: TimestampSchema,
  reason: z.string(),
  created_at: TimestampSchema,
  state: z
    .object({ status: z.enum(["pending", "applied", "rejected", "cancelled"]) })
    .passthrough(),
});
export const ResumeScheduleListSchema = Page(ResumeScheduleSchema);
const EscalationSourceSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("task"),
    task_id: UuidV7Schema,
    pipeline_version_id: UuidV7Schema,
    stage_id: StableKeySchema,
    stage_visit: RevisionSchema,
    wait_condition_id: UuidV7Schema,
  }),
  z.object({
    kind: z.literal("communication"),
    run_id: UuidV7Schema,
    assignment_id: UuidV7Schema,
    thread_id: UuidV7Schema,
    source_message_id: UuidV7Schema,
  }),
]);
export const EscalationSchema = z.object({
  id: UuidV7Schema,
  source: EscalationSourceSchema,
  category: z.enum([
    "action_approval",
    "clarification",
    "scope_or_policy_conflict",
    "stale_or_invalid_task",
    "technical_decision",
    "blocked",
  ]),
  question: z.string(),
  allowed_outcomes: z.array(StableKeySchema).max(128),
  revision: RevisionSchema,
  generation: z.number().int().nonnegative().safe(),
  state: z
    .object({
      status: z.enum(["queued", "assigned", "resolved", "needs_management_change", "superseded"]),
    })
    .passthrough(),
  latest_assignment: z
    .object({
      id: UuidV7Schema,
      generation: RevisionSchema,
      resolver: z.discriminatedUnion("kind", [
        z.object({ kind: z.literal("human") }),
        z.object({ kind: z.literal("employee"), employee_id: UuidV7Schema }),
      ]),
      issued_at: TimestampSchema,
      expires_at: TimestampSchema.nullable(),
      state: z.object({ status: z.enum(["active", "answered", "retired"]) }).passthrough(),
    })
    .nullable(),
  created_at: TimestampSchema,
});
export const EscalationListSchema = Page(EscalationSchema);
export const ResolverRouteSchema = z.object({
  key: StableKeySchema,
  revision: RevisionSchema,
  employee_ids: z.array(UuidV7Schema).max(100),
  assignment_timeout_seconds: z.number().int().positive().safe(),
  updated_at: TimestampSchema,
});
export const ResolverRouteListSchema = z.object({
  items: z.array(ResolverRouteSchema).max(20),
  next_cursor: StableKeySchema.optional(),
});
export const RecoverySettingsSchema = z.object({
  boot_policy: z.enum(["manual_hold", "recover_safe_then_hold", "reconcile_then_resume_queue"]),
  hold: z.boolean(),
});
export const RecoveryRunSchema = z.object({
  run_id: UuidV7Schema,
  task_id: UuidV7Schema.nullable(),
  desired_state: z.enum([
    "provision_requested",
    "running",
    "stop_requested",
    "force_stop_requested",
    "stopped",
    "failed",
  ]),
  observed_state: z.enum([
    "unknown",
    "provisioning",
    "running",
    "stopping",
    "stopped",
    "failed",
    "lost",
  ]),
  created_at: TimestampSchema,
  started_at: TimestampSchema.nullable(),
  liveness_observed_at: TimestampSchema.nullable(),
  stop_requested_at: TimestampSchema.nullable(),
  updated_at: TimestampSchema,
  lease_active: z.boolean(),
  reservation_state: z.string().nullable(),
  reservation_released: z.boolean().nullable(),
  accepted_assessment: z
    .enum(["not_started_confirmed", "partial_work_observed", "external_effect_possible", "unknown"])
    .nullable(),
  assessment_accepted_at: TimestampSchema.nullable(),
  assessment_command_id: UuidV7Schema.nullable(),
  recovery_queue_entry_id: UuidV7Schema.nullable(),
});
export const RecoveryRunListSchema = z.object({
  items: z.array(RecoveryRunSchema).max(20),
  next_cursor: UuidV7Schema.optional(),
});
export const NextRunConstraintSchema = z.object({
  id: UuidV7Schema,
  task_id: UuidV7Schema,
  pipeline_version_id: UuidV7Schema,
  stage_id: StableKeySchema,
  stage_visit: RevisionSchema,
  employee_id: UuidV7Schema,
  created_at: TimestampSchema,
  state: z.discriminatedUnion("status", [
    z.object({ status: z.literal("pending") }),
    z.object({
      status: z.literal("blocked"),
      reason: z.enum(["employee_disabled", "stage_ineligible"]),
    }),
    z.object({ status: z.literal("consumed"), run_id: UuidV7Schema }),
    z.object({
      status: z.literal("cancelled"),
      reason: z.enum(["cleared", "superseded", "stage_left", "task_terminal"]),
    }),
  ]),
});
export const NextRunConstraintListSchema = z.object({
  items: z.array(NextRunConstraintSchema).max(20),
  next_cursor: UuidV7Schema.optional(),
});
export const EscalationDetailSchema = EscalationSchema.extend({
  route: ResolverRouteSchema.nullable(),
  created_by: z.object({ kind: z.string(), id: UuidV7Schema }),
});
export const RecoveryReadinessSchema = z
  .object({
    run_id: UuidV7Schema,
    assessment: z.literal("not_started_confirmed"),
    eligible: z.boolean(),
    reason_code: z
      .enum([
        "already_assessed",
        "recovery_state_unavailable",
        "execution_not_retired",
        "physical_state_unresolved",
        "result_evidence_present",
        "unsupported_assignment",
        "task_stage_changed",
        "recovery_wait_missing",
        "recovery_wait_unresolvable",
        "pipeline_version_unavailable",
        "pipeline_stage_unavailable",
        "executor_not_employee",
      ])
      .nullable(),
    project_revision: RevisionSchema,
    run_revision: RevisionSchema,
    lease_fencing_token: RevisionSchema,
    environment_epoch: RevisionSchema,
    task_id: UuidV7Schema.nullable(),
    task_revision: RevisionSchema.nullable(),
  })
  .superRefine((value, ctx) => {
    if (value.eligible !== (value.reason_code === null))
      ctx.addIssue({ code: z.ZodIssueCode.custom, message: "inconsistent readiness" });
  });
export type ResumeSchedule = z.infer<typeof ResumeScheduleSchema>;
export type Escalation = z.infer<typeof EscalationSchema>;
