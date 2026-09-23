import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("taskless Project configures a typed property schema through a confirmed Core command", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.empty_project.id);
  const properties = page.getByRole("region", { name: "Task properties" });
  await expect(properties).toContainText("No Task properties configured");
  await properties.getByRole("button", { name: "Configure schema" }).click();
  await properties.getByRole("textbox", { name: "Complete schema JSON" }).fill(
    JSON.stringify({
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
    }),
  );
  await properties.getByRole("button", { name: "Review save" }).click();
  await expect(properties).toContainText("Confirm full replacement");
  await properties.getByRole("button", { name: "Confirm replacement" }).click();
  await expect(properties.getByText("Impact")).toBeVisible();
  await expect(properties.getByRole("button", { name: "Configure schema" })).toBeVisible();

  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  const catalog = page.getByRole("region", { name: "Pipeline catalog" });
  await catalog.getByRole("button", { name: "Create Pipeline", exact: true }).click();
  const pipelineEditor = catalog.getByRole("region", { name: "Create Pipeline" });
  await pipelineEditor.getByLabel("Pipeline name").fill("Property test Pipeline");
  await pipelineEditor.getByRole("button", { name: "Create immutable Pipeline" }).click();
  await pipelineEditor.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(catalog).toContainText("Property test Pipeline");

  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByRole("button", { name: "Create draft", exact: true }).click();
  const creation = page.getByRole("region", { name: "Create draft Task", exact: true });
  await creation.getByLabel("Draft title").fill("Typed property draft");
  await creation.getByLabel("Task kind").selectOption("delivery");
  await creation.getByLabel("Task properties JSON").fill(
    JSON.stringify({
      impact: { type: "enum", value: "high" },
    }),
  );
  await creation
    .getByRole("button", { name: /^Select Pipeline version / })
    .first()
    .click();
  await creation.getByRole("button", { name: "Create draft", exact: true }).click();
  const detail = page.getByRole("region", { name: "Task detail", exact: true });
  await expect(detail.getByRole("region", { name: "Properties" })).toContainText("impact (enum)");
  await expect(detail.getByRole("region", { name: "Properties" })).toContainText("high");
  await expect(page.getByRole("region", { name: "Activity", exact: true })).toHaveCount(1);
});
