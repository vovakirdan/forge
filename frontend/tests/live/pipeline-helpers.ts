import type { Locator, Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";
import { loadProject } from "./task-helpers";

export async function openPipelines(page: Page, live: Live) {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.pipelines_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  await expect(
    pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
  ).toHaveCount(20);
}

export function pipelineList(page: Page) {
  return page.getByRole("region", { name: "Pipeline versions", exact: true });
}

export function pipelineCard(page: Page) {
  return page.getByRole("region", { name: "Pipeline version details", exact: true });
}

export function openVersion(page: Page, id: string) {
  return pipelineList(page).getByRole("button", {
    name: `Open Pipeline version ${id}`,
    exact: true,
  });
}

export async function fact(region: Locator, label: string, value: string | RegExp) {
  // Semantic definition-list adjacency; no dependence on layout classes.
  await expect(region.getByText(label, { exact: true }).locator("+ dd")).toHaveText(value);
}
