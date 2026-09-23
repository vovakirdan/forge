import { test, expect } from "./fixtures";
import { dependencyPanel, openDependencies } from "./dependency-helpers";
import { draftForm } from "./draft-helpers";
import { loadProject } from "./task-helpers";

test("owner creates and removes a dependency using a Task beyond the first picker page", async ({
  live,
  page,
}) => {
  await openDependencies(page, live);
  const fixture = live.core.dependency_reads_project;
  await draftForm(page).getByRole("button", { name: "Close task", exact: true }).click();
  await page
    .getByRole("region", { name: "Tasks", exact: true })
    .getByRole("button", { name: `Open ${fixture.empty.key}`, exact: true })
    .click();
  await dependencyPanel(page).getByRole("button", { name: "Add blocker" }).click();
  const editor = page.getByRole("region", { name: "Edit dependency" });
  const candidate = editor.getByRole("button", {
    name: new RegExp(`^${fixture.last_blocked.key} `),
  });
  for (let pageNumber = 0; pageNumber < 6; pageNumber++) {
    await expect(editor.getByRole("list", { name: "Dependency candidates" })).toBeVisible();
    if ((await candidate.count()) > 0) break;
    const next = editor.getByRole("button", { name: "Next" });
    await expect(next).toBeEnabled();
    await next.click();
  }
  await expect(candidate).toBeVisible();
  await candidate.click();
  await expect(editor).toContainText(
    `${fixture.last_blocked.key} must finish before ${fixture.empty.key}`,
  );
  await editor.getByRole("button", { name: "Confirm link" }).click();
  await expect(editor).toContainText("Core accepted the command");
  await expect(dependencyPanel(page)).toContainText(fixture.last_blocked.key);
  await editor.getByRole("button", { name: "Close" }).click();
  await dependencyPanel(page)
    .getByRole("button", { name: `Remove link with ${fixture.last_blocked.key}` })
    .click();
  await expect(editor).toContainText("Removing this link may release a wait");
  await editor.getByRole("button", { name: "Confirm removal" }).click();
  await expect(editor).toContainText("Core accepted the command");
  await expect(dependencyPanel(page)).not.toContainText(fixture.last_blocked.key);
});

test("Blocks direction replays the same command after a lost receipt", async ({ live, page }) => {
  await openDependencies(page, live);
  const fixture = live.core.dependency_reads_project;
  await draftForm(page).getByRole("button", { name: "Close task", exact: true }).click();
  await page
    .getByRole("region", { name: "Tasks", exact: true })
    .getByRole("button", { name: `Open ${fixture.empty.key}`, exact: true })
    .click();
  await dependencyPanel(page, "blocks").getByRole("button", { name: "Block another task" }).click();
  const editor = page.getByRole("region", { name: "Edit dependency" });
  await editor.getByRole("button", { name: new RegExp(`^${fixture.first_blocker.key} `) }).click();
  await expect(editor).toContainText(
    `${fixture.empty.key} must finish before ${fixture.first_blocker.key}`,
  );
  const requests: { body: string | null; key: string | undefined }[] = [];
  await page.route(`${live.origin}/api/commands/create_dependency`, async (route) => {
    requests.push({
      body: route.request().postData(),
      key: route.request().headers()["idempotency-key"],
    });
    const response = await route.fetch({ timeout: 10_000 });
    if (requests.length === 1) await route.fulfill({ status: 200, json: { invalid: true } });
    else await route.fulfill({ response });
  });
  await editor.getByRole("button", { name: "Confirm link" }).click();
  await expect(editor).toContainText("Outcome unknown");
  await editor.getByRole("button", { name: "Retry same command" }).click();
  await expect(editor).toContainText("Core accepted the command (replayed)");
  expect(requests).toHaveLength(2);
  expect(requests[0]?.key).toMatch(/^[0-9a-f-]{36}$/);
  expect(requests[1]).toEqual(requests[0]);
  await expect(dependencyPanel(page, "blocks")).toContainText(fixture.first_blocker.key);
  await editor.getByRole("button", { name: "Close" }).click();
  await dependencyPanel(page, "blocks")
    .getByRole("button", { name: `Remove link with ${fixture.first_blocker.key}` })
    .click();
  await editor.getByRole("button", { name: "Confirm removal" }).click();
  await expect(editor).toContainText("Core accepted the command");
});

test("unsent dependency selection is scoped and guarded on a narrow screen", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openDependencies(page, live);
  const fixture = live.core.dependency_reads_project;
  await dependencyPanel(page).getByRole("button", { name: "Add blocker" }).click();
  const editor = page.getByRole("region", { name: "Edit dependency" });
  const candidate = editor.getByRole("button", {
    name: new RegExp(`^${fixture.first_blocker.key} `),
  });
  await candidate.focus();
  await page.keyboard.press("Enter");
  await expect(editor).toContainText(
    `${fixture.first_blocker.key} must finish before ${fixture.root.key}`,
  );
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/api/commands/create_dependency")) posts++;
  });
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByLabel("Project ID", { exact: true }).fill(live.core.empty_project.id);
  await page.getByRole("button", { name: "Load project", exact: true }).click();
  await expect(editor).toBeVisible();
  page.once("dialog", (dialog) => dialog.accept());
  await loadProject(page, live.core.empty_project.id);
  await expect(editor).toHaveCount(0);
  expect(posts).toBe(0);
});
