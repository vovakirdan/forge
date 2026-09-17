import { z } from "zod";

/** Validate read-only wire data without Zod rebuilding away additive JSON keys. */
export function preserveWireValue<Schema extends z.ZodTypeAny>(schema: Schema) {
  return z.unknown().transform((value, context): z.output<Schema> => {
    const validated = schema.safeParse(value);
    if (!validated.success) {
      for (const issue of validated.error.issues) context.addIssue(issue);
      return z.NEVER;
    }
    // Contract schemas must not coerce, normalize or inject defaults: the valid
    // original is authoritative, including nested '__proto__' JSON members.
    return value as z.output<Schema>;
  });
}
