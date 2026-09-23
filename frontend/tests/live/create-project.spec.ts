import { test, expect } from "./fixtures";

test("owner creates and opens a canonical Project with its reserved identity", async ({
  live,
  page,
}) => {
  await live.login(page);
  const picker = page.getByRole("region", { name: "Project selector" });
  await picker.getByRole("button", { name: "Create Project", exact: true }).click();
  const editor = picker.getByRole("region", { name: "Create Project" });
  const idText = await editor.locator(".font-mono").textContent();
  const id = idText?.match(
    /[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/,
  )?.[0];
  expect(id).toBeTruthy();
  await editor.getByLabel("Project name").fill("Browser-created Project");
  await editor.getByRole("button", { name: "Create Project", exact: true }).click();
  await expect(editor).toContainText("Canonical Project confirmed.");
  await editor.getByRole("button", { name: "Open created Project" }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Project", exact: true })).toContainText(id!);
  await expect(page.getByRole("region", { name: "Project", exact: true })).toContainText(
    "Browser-created Project",
  );
});
