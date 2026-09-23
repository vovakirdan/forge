import { strict as assert } from "node:assert";
import { describe, it } from "node:test";
import { createPropertySchemaAttempt } from "./property-schema-command.ts";

const projectId = "01988000-0000-7000-8000-000000000001";
const key = "f862c4a2-6cd0-4b77-9cf8-554a3c551197";

describe("Project Task property schema command", () => {
  it("freezes one full replacement with Project revision and retry key", () => {
    const schema = {
      definitions: {
        impact: {
          key: "impact",
          display_name: "Impact",
          property_type: "enum",
          required: true,
          default_value: { type: "enum", value: "medium" },
          allowed_choices: ["medium", "high"],
        },
      },
    };
    const attempt = createPropertySchemaAttempt(projectId, 3, schema, key);
    assert.deepEqual(JSON.parse(attempt.body), {
      project_id: projectId,
      expected_revision: 3,
      payload: { schema },
    });
    assert.equal(attempt.key, key);
    assert.equal(Object.isFrozen(attempt), true);
  });

  it("rejects mismatched keys and malformed defaults before sending", () => {
    const base = {
      key: "impact",
      display_name: "Impact",
      property_type: "enum",
      required: true,
      default_value: null,
      allowed_choices: ["high"],
    };
    assert.throws(() =>
      createPropertySchemaAttempt(projectId, 3, { definitions: { other: base } }, key),
    );
    assert.throws(() =>
      createPropertySchemaAttempt(
        projectId,
        3,
        { definitions: { impact: { ...base, default_value: { type: "text", value: "high" } } } },
        key,
      ),
    );
  });
});
