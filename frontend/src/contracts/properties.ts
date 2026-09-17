import { z } from "zod";
import { StableKeySchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const ForgeReferenceSchema = preserveWireValue(
  z
    .object({
      reference_type: z.enum(["project", "task", "artifact", "employee"]),
      reference_id: UuidV7Schema,
    })
    .passthrough(),
);

// Core serializes time::Date as [year, ordinal_day], not an ISO date string.
export const PropertyDateSchema = z.tuple([
  z.number().int().safe(),
  z.number().int().min(1).max(366),
]);

export const TaskPropertyValueSchema = preserveWireValue(
  z.discriminatedUnion("type", [
    z.object({ type: z.literal("boolean"), value: z.boolean() }).passthrough(),
    z.object({ type: z.literal("text"), value: z.string() }).passthrough(),
    z.object({ type: z.literal("number"), value: z.number().finite() }).passthrough(),
    z.object({ type: z.literal("date"), value: PropertyDateSchema }).passthrough(),
    z.object({ type: z.literal("enum"), value: z.string().min(1) }).passthrough(),
    z.object({ type: z.literal("multi_enum"), value: z.array(z.string().min(1)) }).passthrough(),
    z.object({ type: z.literal("reference"), value: ForgeReferenceSchema }).passthrough(),
  ]),
);

export const TaskPropertiesSchema = preserveWireValue(
  z.record(StableKeySchema, TaskPropertyValueSchema),
);
export type ForgeReference = z.infer<typeof ForgeReferenceSchema>;
export type TaskPropertyValue = z.infer<typeof TaskPropertyValueSchema>;
export type TaskProperties = z.infer<typeof TaskPropertiesSchema>;
