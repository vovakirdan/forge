import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { draftForm } from "./draft-helpers";
import { holdRealResponse, loadProject } from "./task-helpers";
import {
  createButton,
  createClient,
  createForm,
  createPath,
  fillCreate,
  openCreate,
} from "./create-command-helpers";

test("confirmed creation survives failed Task readback and retries reads only", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  const title = `Readback ${randomUUID()}`;
  await fillCreate(page, live, title);
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(createPath)) posts += 1;
  });
  const pattern = `${live.origin}/api/projects/${live.core.create_commands_project.id}/tasks/*`;
  await page.route(pattern, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await createButton(page).click();
      await expect(createForm(page)).toContainText("Created Task:");
      await expect(
        page.getByRole("button", { name: "Refresh created data", exact: true }),
      ).toBeVisible();
    },
  );
  expect(posts).toBe(1);
  const matches = (await (await createClient(live)).tasks()).items.filter(
    (task) => task.title === title,
  );
  expect(matches).toHaveLength(1);
  await expect(createForm(page)).toContainText(matches[0]!.id);
  await page.unroute(pattern);
  await page.getByRole("button", { name: "Refresh created data", exact: true }).click();
  await expect(draftForm(page)).toContainText(title);
  expect(posts).toBe(1);
});

test("malformed create receipt after real commit remains unknown until exact replay", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  const title = `Malformed ${randomUUID()}`;
  await fillCreate(page, live, title);
  let posts = 0;
  let failed = false;
  await page.route(`${live.origin}${createPath}`, async (route) => {
    try {
      posts += 1;
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real receipt");
      if (posts === 1) await route.fulfill({ status: 200, json: { status: "applied" } });
      else await route.fulfill({ response });
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await createButton(page).click();
  await expect(
    page.getByRole("button", { name: "Retry same creation", exact: true }),
  ).toBeVisible();
  await expect(createForm(page)).not.toContainText("Created Task:");
  await page.getByRole("button", { name: "Retry same creation", exact: true }).click();
  await expect(draftForm(page)).toContainText(title);
  expect(failed).toBe(false);
  expect(posts).toBe(2);
  expect(
    (await (await createClient(live)).tasks()).items.filter((task) => task.title === title),
  ).toHaveLength(1);
  await page.unrouteAll({ behavior: "wait" });
});

test("unsaved creation warns on navigation and accepted Project switch discards it", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  const title = `Unsaved ${randomUUID()}`;
  await fillCreate(page, live, title);
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveValue(title);
  page.once("dialog", (dialog) => dialog.accept());
  await loadProject(page, live.core.empty_project.id);
  await expect(createForm(page)).toHaveCount(0);
  expect(
    (await (await createClient(live)).tasks()).items.filter((task) => task.title === title),
  ).toHaveLength(0);
});

test("logout during accepted creation discards late response without undoing Core", async ({
  live,
  page,
}) => {
  const client = await createClient(live);
  await openCreate(page, live);
  const title = `Logout ${randomUUID()}`;
  await fillCreate(page, live, title);
  const held = await holdRealResponse(page, `${live.origin}${createPath}`);
  try {
    await createButton(page).click();
    await held.ready;
    held.assertCaptured();
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Log out", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Connect to Forge", exact: true }),
    ).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(createForm(page)).toHaveCount(0);
    expect((await client.tasks()).items.filter((task) => task.title === title)).toHaveLength(1);
  } finally {
    await held.cleanup();
  }
});

test("synthetic retired selection after real conflict is retained but cannot be submitted", async ({
  live,
  page,
}) => {
  const client = await createClient(live);
  await openCreate(page, live);
  await fillCreate(page, live, `Retired ${randomUUID()}`);
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  expect(
    (await client.post(await client.baseline(`Concurrent retirement ${randomUUID()}`))).status,
  ).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await createButton(page).click();
      await expect(
        page.getByRole("button", { name: "Refresh creation baseline", exact: true }),
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
          { id: "normal", display_name: "Normal", rank: 50, retired: false },
        ],
      },
    }),
  );
  await page.getByRole("button", { name: "Refresh creation baseline", exact: true }).click();
  await expect(page.getByLabel("Task priority", { exact: true })).toBeEnabled();
  await expect(page.getByLabel("Task priority", { exact: true })).toHaveValue("high");
  await expect(createButton(page)).toBeDisabled();
  await page.getByLabel("Task priority", { exact: true }).selectOption("normal");
  await expect(createButton(page)).toBeEnabled();
  await page.unroute(url);
});

test("missing priority catalog blocks creation and has explicit read recovery", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.create_commands_project.id);
  const url = `${live.origin}/api/projects/${live.core.create_commands_project.id}/priority-scheme`;
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await page.getByRole("button", { name: "Create draft", exact: true }).click();
      await expect(createForm(page).getByRole("alert")).toBeVisible();
    },
  );
  await page.unroute(url);
  await createForm(page)
    .getByRole("button", { name: /Retry.*baseline/ })
    .click();
  await expect(page.getByLabel("Draft title", { exact: true })).toBeEditable();
  await expect(createButton(page)).toBeDisabled();
});

test("create form supports narrow keyboard submit and empty optional text", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openCreate(page, live);
  const title = `Keyboard ${randomUUID()}`;
  await fillCreate(page, live, title);
  const box = await page.getByLabel("Draft title", { exact: true }).boundingBox();
  expect(box).not.toBeNull();
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(375);
  await createButton(page).focus();
  await page.keyboard.press("Enter");
  await expect(draftForm(page)).toContainText(title);
  expect(
    (await (await createClient(live)).tasks()).items.filter((task) => task.title === title),
  ).toHaveLength(1);
});
