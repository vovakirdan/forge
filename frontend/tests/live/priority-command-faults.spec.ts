import { test, expect } from "./fixtures";
import { draftForm } from "./draft-helpers";
import { holdRealResponse, loadProject } from "./task-helpers";
import {
  openPriority,
  priorityClient,
  priorityCommandPath,
  priorityDraft,
  priorityEditor,
} from "./priority-command-helpers";

test("priority confirmed save survives unavailable reads and refresh never resubmits", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 6);
  const url = `${live.origin}/api/projects/${live.core.priority_commands_project.id}/tasks/${draft.id}`;
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(priorityCommandPath)) posts += 1;
  });
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await page.getByRole("button", { name: "Save priority", exact: true }).click();
      await expect(priorityEditor(page)).toContainText("The save is confirmed.");
      await expect(priorityEditor(page)).toContainText("Last loaded priority: Normal (normal)");
      await expect(priorityEditor(page)).not.toContainText("Current saved priority:");
    },
  );
  expect(posts).toBe(1);
  expect((await (await priorityClient(live)).task(draft.id)).priority).toBe("high");
  await page.unroute(url);
  await page.getByRole("button", { name: "Refresh saved data", exact: true }).click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  await expect(priorityEditor(page)).toContainText("Current saved priority: High (high)");
  expect(posts).toBe(1);
});

test("priority unsaved choice warns on navigation and is not persisted", async ({ live, page }) => {
  const draft = await openPriority(page, live, 7);
  const client = await priorityClient(live);
  const before = await client.task(draft.id);
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(page.getByLabel("Task priority", { exact: true })).toHaveValue("high");
  page.once("dialog", (dialog) => dialog.accept());
  await loadProject(page, live.core.empty_project.id);
  await expect(priorityEditor(page)).toHaveCount(0);
  expect(await client.task(draft.id)).toEqual(before);
});

test("logout during priority save discards late response without undoing Core", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 8);
  const client = await priorityClient(live);
  const before = await client.task(draft.id);
  const held = await holdRealResponse(page, `${live.origin}${priorityCommandPath}`);
  try {
    await page.getByLabel("Task priority", { exact: true }).selectOption("high");
    await page.getByRole("button", { name: "Save priority", exact: true }).click();
    await held.ready;
    held.assertCaptured();
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Log out", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Connect to Forge", exact: true }),
    ).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(priorityEditor(page)).toHaveCount(0);
    expect(await client.task(draft.id)).toMatchObject({
      priority: "high",
      revision: before.revision + 1,
    });
  } finally {
    await held.cleanup();
  }
});

test("malformed priority receipt after real commit remains unknown until replay", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 9);
  let posts = 0;
  let failed = false;
  await page.route(`${live.origin}${priorityCommandPath}`, async (route) => {
    try {
      posts += 1;
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real receipt");
      if (posts === 1)
        await route.fulfill({ status: 200, json: { status: "applied", unexpected: true } });
      else await route.fulfill({ response });
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  await page.getByRole("button", { name: "Save priority", exact: true }).click();
  await expect(page.getByRole("button", { name: "Retry same save", exact: true })).toBeVisible();
  await expect(priorityEditor(page)).not.toContainText("Saved.");
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Retry same save", exact: true }).click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  expect(failed).toBe(false);
  expect(posts).toBe(2);
  expect((await (await priorityClient(live)).task(draft.id)).priority).toBe("high");
  await page.unrouteAll({ behavior: "wait" });
});

test("synthetic retirement after real conflict keeps invalid choice until a valid selection", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 10);
  const client = await priorityClient(live);
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  expect((await client.post(await client.priorityBaseline(draft.id, "low"))).status).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await page.getByRole("button", { name: "Save priority", exact: true }).click();
      await expect(
        page.getByRole("button", { name: "Refresh priority baseline", exact: true }),
      ).toBeVisible();
    },
  );
  const project = await client.project();
  const url = `${live.origin}/api/projects/${project.id}/priority-scheme`;
  await page.route(url, (route) =>
    route.fulfill({
      json: {
        project_id: project.id,
        project_revision: project.revision,
        default_level_id: "normal",
        levels: [
          { id: "high", display_name: "Archived", rank: 100, retired: true },
          { id: "low", display_name: "Low", rank: 10, retired: false },
          { id: "normal", display_name: "Normal", rank: 50, retired: false },
        ],
      },
    }),
  );
  await page.getByRole("button", { name: "Refresh priority baseline", exact: true }).click();
  await expect(page.getByLabel("Task priority", { exact: true })).toBeEnabled();
  await expect(page.getByLabel("Task priority", { exact: true })).toHaveValue("high");
  await expect(
    page.getByLabel("Task priority", { exact: true }).locator('option[value="high"]'),
  ).toBeDisabled();
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toBeDisabled();
  expect((await client.task(draft.id)).priority).toBe("low");
  await page.getByLabel("Task priority", { exact: true }).selectOption("normal");
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toBeEnabled();
  await page.unroute(url);
  await page.getByRole("button", { name: "Save priority", exact: true }).click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  expect((await client.task(draft.id)).priority).toBe("normal");
});

test("unavailable priority baseline blocks only its editor and has explicit recovery", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.priority_commands_project.id);
  const draft = priorityDraft(live, 11);
  await page.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  const url = `${live.origin}/api/projects/${live.core.priority_commands_project.id}/priority-scheme`;
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await page.getByRole("button", { name: "Change priority", exact: true }).click();
      await expect(priorityEditor(page).getByRole("alert")).toBeVisible();
    },
  );
  await expect(draftForm(page)).toContainText(draft.key);
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toHaveCount(0);
  await page.unroute(url);
  await priorityEditor(page)
    .getByRole("button", { name: /Retry.*baseline/ })
    .click();
  await expect(page.getByLabel("Task priority", { exact: true })).toBeEnabled();
});

test("priority editor keyboard and narrow viewport use real options without an implicit write", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openPriority(page, live, 11);
  const selector = page.getByLabel("Task priority", { exact: true });
  await expect(selector.getByRole("option")).toHaveCount(3);
  await selector.focus();
  await page.keyboard.press("Home");
  await expect(selector).toHaveValue("high");
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toBeFocused();
  const box = await selector.boundingBox();
  expect(box).not.toBeNull();
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(375);
  expect((await (await priorityClient(live)).task(priorityDraft(live, 11).id)).priority).toBe(
    "normal",
  );
});
