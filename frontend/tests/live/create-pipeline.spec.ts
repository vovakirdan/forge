import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("owner creates a Pipeline and reads its immutable first graph", async ({ live, page }) => {
  await live.login(page);
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  const catalog = page.getByRole("region", { name: "Pipeline catalog" });
  await expect(catalog).toContainText("No Pipelines on this page.");
  await catalog.getByRole("button", { name: "Create Pipeline", exact: true }).click();
  const editor = catalog.getByRole("region", { name: "Create Pipeline" });
  await editor.getByLabel("Pipeline name").fill("First delivery Pipeline");
  await editor.getByRole("button", { name: "Create immutable Pipeline" }).click();
  await expect(editor.getByRole("alertdialog").getByRole("heading")).toBeFocused();
  await editor.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(editor).toHaveCount(0);
  await expect(catalog).toContainText("Created Pipeline");
  await expect(
    catalog.getByRole("listitem").filter({ hasText: "First delivery Pipeline" }),
  ).toBeVisible();
  const versions = page.getByRole("region", { name: "Pipeline versions" });
  await expect(versions).toContainText("First delivery Pipeline · version 1");
});
