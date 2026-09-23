import { z } from "zod";
import { UuidV7Schema } from "./common.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

const ProjectNameSchema = z
  .string()
  .refine((name) => name.trim().length > 0 && [...name].length <= 240);
const RequestSchema = z
  .object({
    project_id: UuidV7Schema,
    expected_revision: z.literal(0),
    payload: z.object({ name: ProjectNameSchema }).strict(),
  })
  .strict();
export type CreateProjectAttempt = Readonly<{
  body: string;
  key: string;
  projectId: string;
  name: string;
}>;

export function reserveProjectId(now = Date.now()): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let millis = BigInt(now);
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = Number(millis & 0xffn);
    millis >>= 8n;
  }
  bytes[6] = (bytes[6]! & 0x0f) | 0x70;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return UuidV7Schema.parse(
    `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`,
  );
}

export function createProjectAttempt(
  projectId: string,
  name: string,
  key: string = crypto.randomUUID(),
): CreateProjectAttempt {
  const request = RequestSchema.parse({
    project_id: projectId,
    expected_revision: 0,
    payload: { name },
  });
  return Object.freeze({
    body: JSON.stringify(request),
    key: IdempotencyKeySchema.parse(key),
    projectId: request.project_id,
    name: request.payload.name,
  });
}

export const CreateProjectReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: z.literal(1),
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("project"), id: UuidV7Schema }).strict(),
  })
  .strict();
