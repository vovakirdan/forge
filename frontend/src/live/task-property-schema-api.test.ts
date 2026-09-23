import assert from "node:assert/strict";
import test from "node:test";
import { createLiveApi, LiveApiError } from "./api.ts";

const project = "01988000-0000-7000-8000-000000000001";
const other = "01988000-0000-7000-8000-000000000002";

test("property schema read preserves typed definitions and Project revision", async () => {
  const api = createLiveApi(async (input) => {
    assert.equal(String(input), `/api/projects/${project}/task-property-schema`);
    return Response.json({
      project_id: project,
      project_revision: 3,
      schema: {
        definitions: {
          impact: {
            key: "impact",
            display_name: "Impact",
            property_type: "enum",
            required: true,
            default_value: { type: "enum", value: "medium" },
            allowed_choices: ["high", "medium"],
          },
        },
      },
    });
  });
  const view = await api.taskPropertySchema(project, "token", new AbortController().signal);
  assert.equal(view.schema.definitions["impact"]?.property_type, "enum");
  assert.equal(view.project_revision, 3);
});

test("property schema read refuses a foreign Project", async () => {
  const api = createLiveApi(async () =>
    Response.json({ project_id: other, project_revision: 1, schema: { definitions: {} } }),
  );
  await assert.rejects(
    api.taskPropertySchema(project, "token", new AbortController().signal),
    (error) => error instanceof LiveApiError && error.kind === "invalid_response",
  );
});
