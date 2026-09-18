// Custom catalogs are synthetic browser responses, not proof of a Core configuration command.
import { test, expect, type Live } from "./fixtures";
import type { Page } from "@playwright/test";
import { draftForm, fixtureDraft } from "./draft-helpers";
import { loadProject } from "./task-helpers";
import { fact } from "./pipeline-helpers";

function scheme(project: string, count: number, name = "Проектный приоритет 🛠") {
  return {
    project_id: project,
    project_revision: 1,
    default_level_id: "normal",
    levels: Array.from({ length: count }, (_, index) => ({
      id: index === 0 ? "normal" : `level_${index}`,
      display_name: index === 0 ? name : `Level ${index}`,
      rank: -index,
      retired: false,
    })),
  };
}

async function openPriorityDraft(page: Page, live: Live) {
  await loadProject(page, live.core.commands_project.id);
  const draft = fixtureDraft(live, 11);
  await page.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await fact(draftForm(page), "Priority ID", "normal");
}

for (const count of [1, 3, 10]) {
  test(`synthetic ${count}-level catalog renders names without changing Task order`, async ({
    live,
    page,
  }) => {
    const title = "<img src=x onerror=window.priorityInjected=true> 🛠";
    const catalog = scheme(live.core.commands_project.id, count, title);
    await page.route(`**/api/projects/${catalog.project_id}/priority-scheme`, (route) =>
      route.fulfill({ json: catalog }),
    );
    await live.login(page);
    await openPriorityDraft(page, live);
    await fact(draftForm(page), "Priority", title);
    const list = page.getByRole("list", { name: "Task list", exact: true });
    await expect(list.getByText("Priority", { exact: true }).locator("+ dd")).toHaveText(
      Array.from({ length: 13 }, () => title),
    );
    const keys = await list
      .getByRole("button", { name: /^Open TASK-/ })
      .evaluateAll((buttons) => buttons.map((button) => button.getAttribute("aria-label")));
    expect(keys.slice(0, 12)).toEqual(
      live.core.commands_project.drafts.map((draft) => `Open ${draft.key}`),
    );
    expect(await page.evaluate(() => "priorityInjected" in window)).toBe(false);
  });
}

test("synthetic retired and unknown priority IDs never become the default", async ({
  live,
  page,
}) => {
  const catalog = scheme(live.core.commands_project.id, 3);
  catalog.default_level_id = "level_1";
  const historical = catalog.levels[0];
  if (!historical) throw new Error("Missing synthetic historical level");
  historical.retired = true;
  await page.route(`**/api/projects/${catalog.project_id}/priority-scheme`, (route) =>
    route.fulfill({ json: catalog }),
  );
  await live.login(page);
  await openPriorityDraft(page, live);
  await fact(draftForm(page), "Priority", /Проектный приоритет.*retired/);
  catalog.levels = catalog.levels.filter((level) => level.id !== "normal");
  await page.getByRole("button", { name: "Refresh priorities", exact: true }).click();
  await fact(draftForm(page), "Priority", "Name unavailable (ID not in catalog)");
  await fact(draftForm(page), "Priority ID", "normal");
});

for (const failure of [
  { name: "unavailable", status: 503, code: "unavailable", message: /service is available/i },
  { name: "oversized", status: 502, code: "response_too_large", message: /limit/i },
  { name: "malformed", status: 200, code: "invalid", message: /unsupported response/i },
  { name: "wrong Project", status: 200, code: "scope", message: /unsupported response/i },
]) {
  test(`synthetic ${failure.name} catalog preserves Task and draft editor`, async ({
    live,
    page,
  }) => {
    const url = `${live.origin}/api/projects/${live.core.commands_project.id}/priority-scheme`;
    await page.route(url, (route) =>
      route.fulfill({
        status: failure.status,
        json:
          failure.code === "scope"
            ? scheme(live.core.second_project.id, 1)
            : { error: "Unavailable", code: failure.code },
      }),
    );
    await live.login(page);
    const check = async () => {
      await openPriorityDraft(page, live);
      await expect(
        page.getByRole("region", { name: "Tasks", exact: true }).getByRole("alert"),
      ).toContainText(failure.message);
      await fact(draftForm(page), "Priority", "Name unavailable (catalog unavailable)");
      await draftForm(page).getByRole("button", { name: "Edit draft", exact: true }).click();
      await expect(page.getByLabel("Draft title", { exact: true })).toBeEditable();
    };
    if (failure.status === 200) await check();
    else
      await live.allowConsoleErrors(
        new RegExp(
          `^Failed to load resource: the server responded with a status of ${failure.status} `,
        ),
        check,
      );
    await page.unroute(url);
    await page.getByRole("button", { name: "Refresh priorities", exact: true }).click();
    await fact(draftForm(page), "Priority", "Normal");
    await expect(page.getByLabel("Draft title", { exact: true })).toBeEditable();
  });
}

test("failed catalog refresh marks prior names stale and manual recovery restores them", async ({
  live,
  page,
}) => {
  await live.login(page);
  await openPriorityDraft(page, live);
  await fact(draftForm(page), "Priority", "Normal");
  const url = `${live.origin}/api/projects/${live.core.commands_project.id}/priority-scheme`;
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await page.getByRole("button", { name: "Refresh priorities", exact: true }).click();
      await expect(page.getByRole("region", { name: "Tasks", exact: true })).toContainText(
        "Showing stale priorities.",
      );
      await fact(draftForm(page), "Priority", /Normal.*stale catalog/);
      await fact(draftForm(page), "Priority ID", "normal");
    },
  );
  await page.unroute(url);
  await page.getByRole("button", { name: "Refresh priorities", exact: true }).click();
  await fact(draftForm(page), "Priority", "Normal");
});
