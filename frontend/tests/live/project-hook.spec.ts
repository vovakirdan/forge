import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("owner configures an immutable hook version without running it", async ({ live, page }) => {
  await live.login(page);
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  const browser = page.getByRole("region", { name: "Project hook versions" });
  await expect(browser).toContainText("No hook versions configured on this page.");
  const results = browser.getByRole("region", { name: "Hook invocation results" });
  await expect(results).toContainText("No hook invocations on this page.");
  await browser.getByRole("button", { name: "Configure hook version" }).click();
  const input = browser.getByLabel("Hook configuration JSON");
  const definition = JSON.parse(await input.inputValue()) as { name: string };
  definition.name = "Example lint";
  await input.fill(JSON.stringify(definition, null, 2));
  await browser.getByRole("button", { name: "Create immutable hook version" }).click();
  await expect(
    browser.getByRole("alertdialog", { name: "Confirm hook configuration" }),
  ).toBeVisible();
  await browser.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(browser.getByRole("listitem").filter({ hasText: "Example lint" })).toBeVisible();
  await expect(browser).toContainText("This view never runs hooks.");
  await expect(results).toContainText("No hook invocations on this page.");
});
