import { z } from "zod";
import { RevisionSchema, StableKeySchema, UuidV7Schema } from "./common.ts";
import { preserveWireValue } from "./preserve-wire-value.ts";

export const PropertyDefinitionSchema = z
  .object({
    key: StableKeySchema,
    display_name: z.string().min(1).max(200),
    property_type: z.enum(["boolean", "text", "number", "date", "enum", "multi_enum", "reference"]),
    required: z.boolean(),
    default_value: z.unknown().nullable(),
    allowed_choices: z.array(z.string().min(1).max(128)).nullable(),
  })
  .passthrough();

export const ProjectTaskPropertySchemaSchema = preserveWireValue(
  z
    .object({
      project_id: UuidV7Schema,
      project_revision: RevisionSchema,
      schema: z
        .object({ definitions: z.record(StableKeySchema, PropertyDefinitionSchema) })
        .passthrough(),
    })
    .passthrough(),
);

const EditableDefinitionSchema = PropertyDefinitionSchema.strip().superRefine(
  (definition, context) => {
    if (definition.default_value !== null) {
      const value = definition.default_value;
      if (
        typeof value !== "object" ||
        Array.isArray(value) ||
        value === null ||
        !("type" in value) ||
        value.type !== definition.property_type ||
        !("value" in value)
      ) {
        context.addIssue({
          code: "custom",
          message: "Default must be a tagged value matching the property type",
        });
      }
    }
    if (
      definition.allowed_choices !== null &&
      definition.property_type !== "enum" &&
      definition.property_type !== "multi_enum"
    ) {
      context.addIssue({ code: "custom", message: "Choices apply only to enum properties" });
    }
  },
);

export const EditableTaskPropertySchema = z
  .object({ definitions: z.record(StableKeySchema, EditableDefinitionSchema) })
  .strict()
  .superRefine((schema, context) => {
    const entries = Object.entries(schema.definitions);
    if (entries.length > 64) {
      context.addIssue({ code: "custom", message: "At most 64 properties are allowed" });
    }
    for (const [key, definition] of entries) {
      if (key !== definition.key) {
        context.addIssue({
          code: "custom",
          message: `Definition key ${definition.key} does not match ${key}`,
        });
      }
    }
    if (new TextEncoder().encode(JSON.stringify(schema)).byteLength > 65_536) {
      context.addIssue({ code: "custom", message: "Schema is larger than 64 KiB" });
    }
  });

export type EditableTaskPropertySchema = z.infer<typeof EditableTaskPropertySchema>;
