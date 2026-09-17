import type { Page } from "@playwright/test";
import { expect, type Live } from "./fixtures";
import { loadProject } from "./task-helpers";

export async function openRuns(page: Page, live: Live) {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.runs_project.id);
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(
    page.getByRole("button", {
      name: `Open Run ${live.core.runs_project.featured_id}`,
      exact: true,
    }),
  ).toBeVisible();
}

export function runList(page: Page) {
  return page.getByRole("region", { name: "Runs", exact: true });
}

export function runCard(page: Page) {
  return page.getByRole("region", { name: "Run details", exact: true });
}

/** Assert only a Boolean on failure: never let assertion diffs print a bearer. */
export function expectNoPrivateRunFields(body: string, secrets: ReadonlySet<string>) {
  const keys = [
    "run_spec",
    "context_manifest",
    "observed_details",
    "authorization",
    "auth",
    "access_token",
    "prompt",
    "system_prompt",
  ];
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null)
    throw new Error("Run response must be an object");
  const records: unknown[] = "items" in value && Array.isArray(value.items) ? value.items : [value];
  for (const record of records) {
    if (typeof record !== "object" || record === null)
      throw new Error("Run record must be an object");
    for (const key of keys)
      expect(Object.hasOwn(record, key), `No private ${key} field`).toBe(false);
  }
  expect(
    [...secrets].some((secret) => body.includes(secret)),
    "No issued owner secret in Run response",
  ).toBe(false);
  expect(
    body.includes("deterministic_m0_simulator"),
    "Private fake RunSpec body is not public",
  ).toBe(false);
}
