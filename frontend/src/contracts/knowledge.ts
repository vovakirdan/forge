import { z } from "zod";
import { ActorReferenceSchema, RevisionSchema, TimestampSchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";
import { IdempotencyKeySchema } from "./task-command.ts";

export const KnowledgeSourceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("artifact"), artifact_id: UuidV7Schema }).passthrough(),
  z.object({ kind: z.literal("event"), event_id: UuidV7Schema }).passthrough(),
  z.object({ kind: z.literal("task_handoff"), handoff_id: UuidV7Schema }).passthrough(),
  z
    .object({ kind: z.literal("knowledge_page"), page_id: UuidV7Schema, revision: RevisionSchema })
    .passthrough(),
]);

export const KnowledgePageSchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      project_id: UuidV7Schema,
      revision: RevisionSchema,
      kind: z.enum(["introduction", "architecture", "guide", "policy", "decision"]),
      status: z.enum(["draft", "published", "withdrawn"]),
      content: z
        .object({
          title: z.string(),
          markdown: z.string(),
          source_refs: z.array(KnowledgeSourceSchema),
        })
        .passthrough(),
      content_hash: z.string().regex(/^[0-9a-f]{64}$/),
      actor: ActorReferenceSchema,
      command_id: UuidV7Schema,
      created_at: TimestampSchema,
      revised_at: TimestampSchema,
      supersedes_revision: RevisionSchema.nullable(),
    })
    .passthrough(),
);
const KnowledgePageKindSchema = z.enum([
  "introduction",
  "architecture",
  "guide",
  "policy",
  "decision",
]);

export const DerivedMemoryEntrySchema = preserveWireValue(
  z
    .object({
      id: UuidV7Schema,
      project_id: UuidV7Schema,
      revision: RevisionSchema,
      subject: z.discriminatedUnion("kind", [
        z.object({ kind: z.literal("task_summary"), task_id: UuidV7Schema }).passthrough(),
        z
          .object({
            kind: z.literal("employee_memory_entry"),
            employee_id: UuidV7Schema,
            task_id: UuidV7Schema.nullable(),
          })
          .passthrough(),
        z
          .object({ kind: z.literal("project_knowledge_entry"), task_id: UuidV7Schema.nullable() })
          .passthrough(),
      ]),
      markdown: z.string(),
      source_refs: z.array(KnowledgeSourceSchema),
      content_hash: z.string().regex(/^[0-9a-f]{64}$/),
      coverage: z
        .object({ first_event_sequence: RevisionSchema, last_event_sequence: RevisionSchema })
        .passthrough()
        .nullable(),
      created_by_job_id: UuidV7Schema,
      created_at: TimestampSchema,
      withdrawn: z.boolean(),
    })
    .passthrough(),
);

export const KnowledgePageListSchema = preserveWireValue(
  z
    .object({ items: z.array(KnowledgePageSchema), next_cursor: UuidV7Schema.nullable() })
    .passthrough(),
);
export const KnowledgeHistorySchema = preserveWireValue(
  z
    .object({
      items: z.array(KnowledgePageSchema),
      next_cursor: z.string().max(19).regex(/^\d+$/).nullable(),
    })
    .passthrough(),
);
export const DerivedMemoryListSchema = preserveWireValue(
  z
    .object({ items: z.array(DerivedMemoryEntrySchema), next_cursor: UuidV7Schema.nullable() })
    .passthrough(),
);
export const DerivedMemoryHistorySchema = preserveWireValue(
  z
    .object({
      items: z.array(DerivedMemoryEntrySchema),
      next_cursor: z.string().max(19).regex(/^\d+$/).nullable(),
    })
    .passthrough(),
);

export const MemorySearchResultSchema = preserveWireValue(
  z
    .object({
      project_id: UuidV7Schema,
      employee_id: UuidV7Schema.nullable(),
      mode: z.enum(["indexed", "canonical_fallback"]),
      degradation: z
        .enum(["not_configured", "index_unavailable", "rejected_projection"])
        .nullable(),
      truncated: z.boolean(),
      results: z.array(
        z
          .object({
            projection_id: UuidV7Schema.nullable(),
            score: z.number().nullable(),
            document: z.discriminatedUnion("kind", [
              z
                .object({ kind: z.literal("knowledge_page"), record: KnowledgePageSchema })
                .passthrough(),
              z
                .object({ kind: z.literal("derived_memory"), record: DerivedMemoryEntrySchema })
                .passthrough(),
            ]),
          })
          .passthrough(),
      ),
    })
    .passthrough(),
);

export const MemoryProjectionStatusSchema = preserveWireValue(
  z
    .object({
      indexed: z.number().int().nonnegative(),
      pending: z.number().int().nonnegative(),
      retired: z.number().int().nonnegative(),
      failed_attempts: z.number().int().nonnegative(),
      canonical_max_revision: z.number().int().nonnegative(),
      configured: z.boolean(),
    })
    .passthrough(),
);

export const EmployeeOnboardingStatusSchema = preserveWireValue(
  z
    .object({
      employee_id: UuidV7Schema,
      state: z.enum(["pending", "completed", "skipped", "legacy_bypass"]),
      revision: RevisionSchema,
      job_id: UuidV7Schema.nullable(),
      receipt: z.unknown().nullable(),
    })
    .passthrough(),
);

export type KnowledgeSource = z.infer<typeof KnowledgeSourceSchema>;
export type KnowledgePage = z.infer<typeof KnowledgePageSchema>;
export type DerivedMemoryEntry = z.infer<typeof DerivedMemoryEntrySchema>;

const SourceInputSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("artifact"), artifact_id: UuidV7Schema }).strict(),
  z.object({ kind: z.literal("event"), event_id: UuidV7Schema }).strict(),
  z.object({ kind: z.literal("task_handoff"), handoff_id: UuidV7Schema }).strict(),
  z
    .object({ kind: z.literal("knowledge_page"), page_id: UuidV7Schema, revision: RevisionSchema })
    .strict(),
]);
const SourcesInputSchema = z
  .array(SourceInputSchema)
  .max(128)
  .refine(
    (sources) => new Set(sources.map((source) => JSON.stringify(source))).size === sources.length,
  );
const pageId = UuidV7Schema;
const revision = RevisionSchema.max(Number.MAX_SAFE_INTEGER - 1);
const content = {
  title: z
    .string()
    .refine((value) => value.trim().length > 0 && new TextEncoder().encode(value).length <= 200),
  markdown: z
    .string()
    .refine((value) => value.trim().length > 0 && new TextEncoder().encode(value).length <= 65_536),
  source_refs: SourcesInputSchema,
};
const identity = { page_id: pageId, expected_page_revision: revision };
const payloads = {
  author_knowledge_page: z
    .object({
      page_id: pageId,
      expected_page_revision: z
        .number()
        .int()
        .min(0)
        .max(Number.MAX_SAFE_INTEGER - 1),
      kind: KnowledgePageKindSchema,
      ...content,
    })
    .strict(),
  publish_knowledge_page: z.object(identity).strict(),
  supersede_knowledge_page: z.object({ ...identity, ...content }).strict(),
  withdraw_knowledge_page: z.object(identity).strict(),
};
export const KnowledgeActionSchema = z.enum([
  "author_knowledge_page",
  "publish_knowledge_page",
  "supersede_knowledge_page",
  "withdraw_knowledge_page",
]);
export type KnowledgeAction = z.infer<typeof KnowledgeActionSchema>;
export type KnowledgeAttempt = Readonly<{
  action: KnowledgeAction;
  body: string;
  key: string;
  pageId: string;
}>;
export const KnowledgeReceiptSchema = z
  .object({
    command_id: UuidV7Schema,
    status: z.enum(["applied", "replayed"]),
    project_revision: RevisionSchema,
    event_ids: z.array(UuidV7Schema).min(1),
    resource: z.object({ kind: z.literal("knowledge_page"), id: UuidV7Schema }).strict(),
  })
  .strict();
export type KnowledgeReceipt = z.infer<typeof KnowledgeReceiptSchema>;

export function knowledgeAttempt(
  action: KnowledgeAction,
  request: { project_id: string; expected_revision: number; payload: unknown },
  key: string = crypto.randomUUID(),
): KnowledgeAttempt {
  const selected = KnowledgeActionSchema.parse(action);
  const project = UuidV7Schema.parse(request.project_id);
  const expected = revision.parse(request.expected_revision);
  const payload = payloads[selected].parse(request.payload);
  return Object.freeze({
    action: selected,
    body: JSON.stringify({ project_id: project, expected_revision: expected, payload }),
    key: IdempotencyKeySchema.parse(key),
    pageId: UuidV7Schema.parse((payload as { page_id: unknown }).page_id),
  });
}

/** RFC 9562 UUIDv7 for reserving a canonical page identity before its first command. */
export function reserveKnowledgePageId(now = Date.now()): string {
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
