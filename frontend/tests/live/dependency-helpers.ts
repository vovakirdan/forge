import type { Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";
import { loadProject } from "./task-helpers";

export type Direction = "blocked_by" | "blocks";
export const dependencyPanel = (page: Page, direction: Direction = "blocked_by") =>
  page.getByRole("region", {
    name: direction === "blocked_by" ? "Depends on" : "Blocks",
    exact: true,
  });
export const relatedButtons = (page: Page, direction: Direction = "blocked_by") =>
  dependencyPanel(page, direction).getByRole("button", { name: /^Open related TASK-/ });
export const dependencyUrl = (live: Live, direction: Direction = "blocked_by") =>
  `${live.origin}/api/projects/${live.core.dependency_reads_project.id}/tasks/${live.core.dependency_reads_project.root.id}/dependencies/${direction}`;

export async function openDependencies(page: Page, live: Live) {
  await live.login(page);
  await loadProject(page, live.core.dependency_reads_project.id);
  await page
    .getByRole("region", { name: "Tasks", exact: true })
    .getByRole("button", {
      name: `Open ${live.core.dependency_reads_project.root.key}`,
      exact: true,
    })
    .click();
  await expect(relatedButtons(page)).toHaveCount(20);
  await expect(relatedButtons(page, "blocks")).toHaveCount(20);
}
