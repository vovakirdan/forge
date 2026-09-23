import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("runtime form rejects secret material before issuing a command", async ({ live, page }) => {
  await live.login(page);
  await loadProject(page, live.core.team_project.id);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page
    .getByRole("button", { name: /Open Employee/ })
    .first()
    .click();
  const panel = page.getByRole("region", { name: "Employee runtime configuration" });
  await expect(panel).toContainText("No runtime binding configured.");
  await panel.getByRole("button", { name: "Configure runtime" }).click();
  await panel.getByLabel("Runtime binding JSON").fill(JSON.stringify({ api_key: "must-not-pass" }));
  await panel.getByRole("button", { name: "Review runtime configuration" }).click();
  await panel.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(panel.getByRole("alert")).toContainText("secret-free RuntimeBinding");
  await expect(panel).toContainText("Current binding:");
});
