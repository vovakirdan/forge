import { type Locator, type Page } from "@playwright/test";
import { expect, test } from "./fixtures";

const taskTitle = /^TASK-142 — Track partial moves/;

async function expectProjectReady(page: Page) {
  // This label is supplied by the client query, unlike SSR navigation links.
  await expect(
    page.getByRole("complementary").getByRole("button", { name: /^Surge Compiler/ }),
  ).toBeVisible();
}

async function expectWithinViewport(dialog: Locator, page: Page) {
  await expect(dialog).toBeVisible();
  // Retry while the dialog's opening animation changes its bounding rectangle.
  await expect(async () => {
    const box = await dialog.boundingBox();
    const viewport = page.viewportSize();
    expect(box).not.toBeNull();
    expect(viewport).not.toBeNull();
    if (box && viewport) {
      expect(box.x).toBeGreaterThanOrEqual(-1);
      expect(box.y).toBeGreaterThanOrEqual(-1);
      expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 1);
      expect(box.y + box.height).toBeLessThanOrEqual(viewport.height + 1);
    }
  }).toPass();
}

test("main demo screens navigate and reload", async ({ page }) => {
  await page.goto("/");
  await expectProjectReady(page);
  const pages = [
    { link: "Overview", path: "/", heading: "Control Room" },
    { link: "Board", path: "/board", heading: "Engineering Board" },
    { link: "Team", path: "/team", heading: "Team" },
    { link: "Knowledge", path: "/knowledge", heading: "Knowledge" },
  ];
  for (const destination of pages) {
    await page
      .getByRole("navigation")
      .getByRole("link", { name: destination.link, exact: true })
      .click();
    await expect(page).toHaveURL(destination.path);
    await expect(
      page.getByRole("heading", { name: destination.heading, exact: true }),
    ).toBeVisible();
    await page.reload();
    await expectProjectReady(page);
    await expect(
      page.getByRole("heading", { name: destination.heading, exact: true }),
    ).toBeVisible();
    await expect(page).toHaveURL(destination.path);
  }
});

test("Task drawer opens by keyboard and Escape restores the originating card", async ({ page }) => {
  await page.goto("/board");
  await expectProjectReady(page);
  const card = page.getByRole("button", { name: /^TASK-142 / });
  await card.focus();
  await page.keyboard.press("Enter");
  const drawer = page.getByRole("dialog", { name: taskTitle });
  await expect(drawer).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden();
  await expect(card).toBeFocused();
});

test("Task drawer Full page navigates and reloads without returning focus to Board", async ({
  page,
}) => {
  await page.goto("/board");
  await expectProjectReady(page);
  await page.getByRole("button", { name: /^TASK-142 / }).click();
  const drawer = page.getByRole("dialog", { name: taskTitle });
  await expect(drawer).toBeVisible();
  await drawer.getByRole("link", { name: "Full page", exact: true }).click();
  await expect(page).toHaveURL("/tasks/TASK-142");
  await expect(page.getByRole("heading", { name: taskTitle })).toBeVisible();
  await expect(drawer).toBeHidden();
  await expect(page.getByRole("button", { name: /^TASK-142 / })).toHaveCount(0);
  await expect(
    page.getByRole("navigation").getByRole("link", { name: "Board", exact: true }),
  ).not.toBeFocused();
  await page.reload();
  await expectProjectReady(page);
  await expect(page).toHaveURL("/tasks/TASK-142");
  await expect(page.getByRole("heading", { name: taskTitle })).toBeVisible();
});

test("command palette searches pages and Tasks with keyboard selection", async ({ page }) => {
  await page.goto("/");
  await expectProjectReady(page);
  await expect(page.getByRole("heading", { name: "Control Room", exact: true })).toBeVisible();
  for (const destination of [
    { query: "page Knowledge", option: "Knowledge", path: "/knowledge", heading: /^Knowledge$/ },
    { query: "TASK-142", option: /^TASK-142 /, path: "/tasks/TASK-142", heading: taskTitle },
  ]) {
    await page.keyboard.press("Control+k");
    const dialog = page.getByRole("dialog", { name: "Command palette", exact: true });
    await expect(dialog).toBeVisible();
    const input = dialog.getByRole("combobox", { name: "Search commands", exact: true });
    await expect(input).toBeFocused();
    await input.fill(destination.query);
    await expect(
      dialog.getByRole("option", { name: destination.option, exact: true }),
    ).toBeVisible();
    await input.press("Enter");
    await expect(dialog).toBeHidden();
    await expect(page).toHaveURL(destination.path);
    await expect(page.getByRole("heading", { name: destination.heading })).toBeVisible();
  }
});

test("command palette reports no match and restores focus on Escape", async ({ page }) => {
  await page.goto("/");
  await expectProjectReady(page);
  const opener = page.getByRole("button", { name: /Search tasks, agents, runs/ });
  await opener.focus();
  await page.keyboard.press("Control+k");
  const dialog = page.getByRole("dialog", { name: "Command palette", exact: true });
  await expect(dialog).toBeVisible();
  await dialog
    .getByRole("combobox", { name: "Search commands", exact: true })
    .fill("no_such_demo_entity_9e22");
  await expect(dialog.getByText("No results.", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
});

test.describe("narrow viewport dialogs", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("command palette fits, searches and closes", async ({ page }) => {
    await page.goto("/");
    await expectProjectReady(page);
    await expect(page.getByRole("heading", { name: "Control Room", exact: true })).toBeVisible();
    await page.keyboard.press("Control+k");
    const dialog = page.getByRole("dialog", { name: "Command palette", exact: true });
    await expectWithinViewport(dialog, page);
    await dialog.getByRole("combobox", { name: "Search commands", exact: true }).fill("page Team");
    await expect(dialog.getByRole("option", { name: "Team", exact: true })).toBeVisible();
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(dialog).toBeHidden();
  });

  test("Task drawer fits and its close button is usable", async ({ page }) => {
    await page.goto("/board");
    await expectProjectReady(page);
    await page.getByRole("button", { name: /^TASK-142 / }).click();
    const drawer = page.getByRole("dialog", { name: taskTitle });
    await expectWithinViewport(drawer, page);
    await drawer.getByRole("button", { name: "Close", exact: true }).click();
    await expect(drawer).toBeHidden();
  });
});
