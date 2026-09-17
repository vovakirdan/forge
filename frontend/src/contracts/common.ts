import { z } from "zod";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const UuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
export const StableKeySchema = z.string().regex(/^[a-z][a-z0-9_]{0,63}$/);
export const TimestampSchema = z.string().datetime({ offset: true });
// JSON numbers cannot retain revisions beyond JavaScript's exact integer range.
export const RevisionSchema = z.number().int().positive().safe();
export const TaskKindSchema = z.enum(["delivery", "analysis"]);
export const LifecycleStatusSchema = z.enum([
  "draft",
  "ready",
  "in_progress",
  "waiting",
  "done",
  "cancelled",
]);

export const ActorReferenceSchema = preserveWireValue(
  z
    .object({
      kind: z.enum(["human", "employee", "system_manager", "core", "supervisor"]),
      id: UuidV7Schema,
    })
    .passthrough(),
);

type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

function isJsonValue(value: unknown, ancestors = new Set<object>()): value is JsonValue {
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value !== "object" || ancestors.has(value)) return false;
  const prototype: unknown = Object.getPrototypeOf(value);
  if (!Array.isArray(value) && prototype !== Object.prototype && prototype !== null) return false;
  ancestors.add(value);
  const children: unknown[] = Array.isArray(value) ? [...value] : Object.values(value);
  const valid = children.every((child) => isJsonValue(child, ancestors));
  ancestors.delete(value);
  return valid;
}

// Validate without rebuilding objects: Zod records strip legitimate '__proto__' keys.
// JSON remains data, not safe HTML, a trusted URL, or an object to merge into config.
export const JsonValueSchema = z.custom<JsonValue>((value) => isJsonValue(value), {
  message: "Expected a JSON value",
});
export const JsonObjectSchema = JsonValueSchema.refine(
  (value): value is { [key: string]: JsonValue } =>
    value !== null && typeof value === "object" && !Array.isArray(value),
  { message: "Expected a JSON object" },
);

export type TaskKind = z.infer<typeof TaskKindSchema>;
export type LifecycleStatus = z.infer<typeof LifecycleStatusSchema>;
export type ActorReference = z.infer<typeof ActorReferenceSchema>;
