import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const Configure = z
  .object({
    route_key: StableKeySchema,
    employee_ids: z
      .array(UuidV7Schema)
      .max(32)
      .refine((ids) => new Set(ids.map((id) => id.toLowerCase())).size === ids.length),
    assignment_timeout_seconds: z.number().int().min(30).max(86_400),
  })
  .strict();
const Raise = z
  .object({
    task_id: UuidV7Schema,
    expected_task_revision: RevisionSchema,
    route_key: StableKeySchema.nullable(),
    category: z.enum([
      "action_approval",
      "clarification",
      "scope_or_policy_conflict",
      "stale_or_invalid_task",
      "technical_decision",
      "blocked",
    ]),
    question: z
      .string()
      .refine(
        (value) => value.trim().length > 0 && !value.includes("\0") && [...value].length <= 20_000,
      ),
  })
  .strict();
export type ResolverAction = "configure_resolver_route" | "raise_escalation";
export type ResolverAttempt = Readonly<{
  action: ResolverAction;
  body: string;
  key: string;
  projectId: string;
  expectedRevision: number;
  routeKey: string | null;
  taskId: string | null;
}>;
export function resolverAttempt(
  action: ResolverAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): ResolverAttempt {
  const projectId = UuidV7Schema.parse(request.project_id);
  const expectedRevision = RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1).parse(
    request.expected_revision,
  );
  const payload =
    action === "configure_resolver_route"
      ? Configure.parse(request.payload)
      : Raise.parse(request.payload);
  return Object.freeze({
    action,
    body: JSON.stringify({ project_id: projectId, expected_revision: expectedRevision, payload }),
    key: IdempotencyKeySchema.parse(key),
    projectId,
    expectedRevision,
    routeKey: "route_key" in payload ? payload.route_key : null,
    taskId: "task_id" in payload ? payload.task_id : null,
  });
}
export const ResolverReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.enum(["project", "escalation"]), id: UuidV7Schema }).strict(),
  })
  .strict();
