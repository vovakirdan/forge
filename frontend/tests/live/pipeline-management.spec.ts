import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("Pipeline catalog publishes, changes default, and preserves deleted history", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.pipelines_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  const catalog = page.getByRole("region", { name: "Pipeline catalog" });
  const row = catalog.getByRole("listitem").filter({ hasText: "Migration strategy catalog" });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Manage Pipeline" }).click();
  const editor = row.getByRole("region", {
    name: `Manage Pipeline ${live.core.pipelines_project.pipeline_id}`,
  });
  await editor
    .getByLabel("Existing version ID to make default")
    .fill(live.core.pipelines_project.second_id);
  await editor.getByRole("button", { name: "Set default version" }).click();
  await editor.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(editor).toHaveCount(0);
  await expect(row).toContainText(live.core.pipelines_project.second_id);

  await row.getByRole("button", { name: "Manage Pipeline" }).click();
  await editor.getByRole("button", { name: "Copy source version graph into editor" }).click();
  const graph = editor.getByLabel("Complete new graph JSON");
  const definition = JSON.parse(await graph.inputValue()) as { stages: { name: string }[] };
  definition.stages[0]!.name = "Revised migration strategy";
  await graph.fill(JSON.stringify(definition));
  await editor.getByRole("button", { name: "Publish new version" }).click();
  await expect(editor.getByRole("alertdialog").getByRole("heading")).toBeFocused();
  await editor.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(editor).toHaveCount(0);
  await expect(row).toContainText("Latest version");
  await expect(row).toContainText("3 ·");
  await expect(row).toContainText(live.core.pipelines_project.second_id);

  await row.getByRole("button", { name: "Manage Pipeline" }).click();
  await editor.getByRole("button", { name: "Delete Pipeline" }).click();
  await editor.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(editor).toHaveCount(0);
  await expect(row).toContainText("Deleted");
  await expect(row.getByRole("button", { name: "Manage Pipeline" })).toHaveCount(0);
});
