import type { Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";

export async function loadProject(page: Page, projectId: string) {
  const picker = page.getByRole("region", { name: "Project selector" });
  const list = picker.getByRole("list", { name: "Projects" });
  const previous = picker.getByRole("button", { name: "Previous Projects" });
  for (let pageNumber = 0; pageNumber < 10; pageNumber++) {
    await expect(list).toBeVisible();
    if (await previous.isDisabled()) break;
    await previous.click();
    await expect(list).toBeVisible();
  }
  for (let pageNumber = 0; pageNumber < 10; pageNumber++) {
    await expect(list).toBeVisible();
    const choice = list.getByRole("button", { name: new RegExp(projectId) });
    if (await choice.count()) {
      await choice.click();
      await expect(page.getByRole("region", { name: "Project", exact: true })).toContainText(
        projectId,
      );
      return;
    }
    const next = picker.getByRole("button", { name: "Next Projects" });
    await expect(next).toBeEnabled();
    await next.click();
    await expect(picker).toContainText(`Page ${pageNumber + 2}`);
  }
  throw new Error("Project is absent from the first ten pages");
}

export async function openTasks(page: Page, live: Live) {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.project_id);
  await expect(
    page.getByRole("button", { name: `Open ${live.core.tasks.featured_key}`, exact: true }),
  ).toBeVisible();
}

// Only delay a real response. Catch route exceptions locally because Playwright
// may include Authorization in diagnostics; report a finite assertion instead.
export async function holdRealResponse(page: Page, url: string) {
  let release!: () => void;
  let captured!: () => void;
  let finished!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const ready = new Promise<void>((resolve) => {
    captured = resolve;
  });
  const delivered = new Promise<void>((resolve) => {
    finished = resolve;
  });
  let cancelled = false;
  let failed = false;
  const onFailed = (request: import("@playwright/test").Request) => {
    if (request.url() === url && request.failure()?.errorText === "net::ERR_ABORTED")
      cancelled = true;
  };
  page.on("requestfailed", onFailed);
  await page.route(url, async (route) => {
    try {
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) {
        failed = true;
        captured();
        return;
      }
      captured();
      await held;
      try {
        await route.fulfill({ response });
      } catch {
        if (!cancelled) failed = true;
      }
    } catch {
      failed = true;
      captured();
    } finally {
      finished();
    }
  });
  return {
    ready,
    assertCaptured: () => expect(failed, "A real gateway response was captured").toBe(false),
    async releaseAfterCancellation() {
      await expect.poll(() => cancelled).toBe(true);
      release();
      await delivered;
      expect(failed, "Delayed delivery did not fail independently of cancellation").toBe(false);
    },
    async cleanup() {
      release();
      await page.unrouteAll({ behavior: "wait" });
      page.off("requestfailed", onFailed);
    },
  };
}
