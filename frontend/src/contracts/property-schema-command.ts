import { z } from "zod";
import { RevisionSchema, UuidV7Schema } from "./common.ts";
import { EditableTaskPropertySchema } from "./task-property-schema.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

export const ConfigureTaskPropertySchemaRequest = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1),
    payload: z.object({ schema: EditableTaskPropertySchema }).strict(),
  })
  .strict();

export const PropertySchemaReceipt = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("project"), id: UuidV7Schema }).strict(),
  })
  .strict();

export type PropertySchemaAttempt = Readonly<{ body: string; key: string; projectId: string }>;

export function createPropertySchemaAttempt(
  projectId: string,
  projectRevision: number,
  schema: unknown,
  key: string = crypto.randomUUID(),
): PropertySchemaAttempt {
  const request = ConfigureTaskPropertySchemaRequest.parse({
    project_id: projectId,
    expected_revision: projectRevision,
    payload: { schema },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    projectId: request.project_id,
  });
}
