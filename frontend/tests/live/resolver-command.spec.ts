import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("owner configures a Human fallback route and raises a Task escalation", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.project_id);
  await page.getByRole("button", { name: "Management", exact: true }).click();
  const panel = page.getByRole("region", { name: "Resolver route and escalation" });
  await panel.getByLabel("Route key").fill("ui_review");
  await panel.getByRole("button", { name: "Review resolver action" }).click();
  await expect(panel.getByRole("alertdialog", { name: "Confirm resolver action" })).toBeVisible();
  await panel.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(panel.getByLabel("Route key")).toHaveValue("");
  await expect(page.getByRole("region", { name: "Resolver routes" })).toContainText("ui_review");

  await panel.getByLabel("Action").selectOption("raise_escalation");
  await panel.getByLabel("Route key").fill("ui_review");
  await panel.getByLabel("Task ID").fill(live.core.tasks.second_id);
  await panel.getByLabel("Question").fill("Which direction should we take?");
  await panel.getByRole("button", { name: "Review resolver action" }).click();
  await expect(panel.getByRole("alertdialog", { name: "Confirm resolver action" })).toBeVisible();
  await panel.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(panel.getByLabel("Task ID")).toHaveValue("");
  await expect(page.getByRole("region", { name: "Escalations" })).toContainText(
    "Which direction should we take?",
  );
});
