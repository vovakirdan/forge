import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("canonical Knowledge page moves through draft, publication, supersede and withdrawal", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Knowledge", exact: true }).click();
  const pages = page.getByRole("region", { name: "Canonical pages" });
  await pages.getByRole("button", { name: "Create draft" }).click();
  const editor = page.getByRole("region", { name: "Edit canonical page" });
  await editor.getByLabel("Title").fill("Operator guide");
  await editor.getByLabel("Markdown").fill("First canonical revision.");
  await editor.getByRole("button", { name: "Create draft", exact: true }).click();
  await expect(editor).toHaveCount(0);
  const detail = page.getByRole("region", { name: "Knowledge record details" });
  await expect(detail).toContainText("First canonical revision.");
  await expect(detail).toContainText("draft");

  await detail.getByRole("button", { name: "Manage page revisions" }).click();
  await editor.getByRole("button", { name: "Publish", exact: true }).click();
  const confirmation = page.getByRole("alertdialog");
  await expect(confirmation.getByRole("heading")).toBeFocused();
  await confirmation.getByRole("button", { name: "Confirm" }).click();
  await expect(editor).toHaveCount(0);
  await expect(detail).toContainText("published");

  await detail.getByRole("button", { name: "Manage page revisions" }).click();
  await editor.getByLabel("Markdown").fill("Updated canonical revision.");
  await editor.getByRole("button", { name: "Supersede published page" }).click();
  await expect(editor).toHaveCount(0);
  await expect(detail).toContainText("Updated canonical revision.");

  await detail.getByRole("button", { name: "Manage page revisions" }).click();
  await editor.getByRole("button", { name: "Withdraw" }).click();
  await confirmation.getByRole("button", { name: "Confirm" }).click();
  await expect(editor).toHaveCount(0);
  await expect(detail).toContainText("withdrawn");
  await expect(detail.getByRole("button", { name: "Manage page revisions" })).toHaveCount(0);
});
