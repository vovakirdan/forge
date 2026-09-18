import { test, expect } from "./fixtures";
import { commandClient, draftForm } from "./draft-helpers";
import { dependencyPanel, openDependencies, relatedButtons } from "./dependency-helpers";

test("real dependencies page both directions, preserve condition states, and navigate outside the Task list", async ({
  live,
  page,
}) => {
  const fixture = live.core.dependency_reads_project;
  const client = await commandClient(live, { projectId: fixture.id });
  const before = await client.project();
  const rootBefore = await client.task(fixture.root.id);
  await openDependencies(page, live);
  await expect(dependencyPanel(page)).toContainText("Pending");
  await expect(dependencyPanel(page)).toContainText("Satisfied");
  await expect(dependencyPanel(page)).toContainText("Blocker cancelled");
  await expect(dependencyPanel(page)).toContainText("task_done");
  expect(rootBefore.lifecycle).toBe("draft");
  for (const direction of ["blocked_by", "blocks"] as const) {
    await dependencyPanel(page, direction)
      .getByRole("button", { name: "Next dependencies", exact: true })
      .click();
    await expect(relatedButtons(page, direction)).toHaveCount(3);
    await expect(
      dependencyPanel(page, direction).getByRole("button", {
        name: "Next dependencies",
        exact: true,
      }),
    ).toBeDisabled();
  }
  // The final blocked Task cannot be on the first 20-row Task list page.
  const related = fixture.last_blocked;
  const list = page.getByRole("region", { name: "Tasks", exact: true });
  await expect(list.getByRole("button", { name: `Open ${related.key}`, exact: true })).toHaveCount(
    0,
  );
  await dependencyPanel(page, "blocks")
    .getByRole("button", { name: `Open related ${related.key}`, exact: true })
    .click();
  await expect(draftForm(page)).toContainText(related.title);
  await expect(dependencyPanel(page)).toContainText(fixture.root.title);
  await page.getByRole("button", { name: "Back to previous task", exact: true }).click();
  await expect(draftForm(page)).toContainText(fixture.root.title);
  await draftForm(page).getByRole("button", { name: "Close task", exact: true }).click();
  await expect(
    list.getByRole("button", { name: `Open ${fixture.root.key}`, exact: true }),
  ).toBeFocused();
  await list.getByRole("button", { name: `Open ${fixture.empty.key}`, exact: true }).click();
  await expect(relatedButtons(page)).toHaveCount(0);
  await expect(relatedButtons(page, "blocks")).toHaveCount(0);
  await expect(dependencyPanel(page)).toContainText(/no /i);
  await expect(dependencyPanel(page, "blocks")).toContainText(/no /i);
  expect((await client.project()).revision).toBe(before.revision);
  expect((await client.task(fixture.root.id)).revision).toBe(rootBefore.revision);
  expect((await client.task(fixture.root.id)).lifecycle).toBe("draft");
  expect((await client.runs()).items).toEqual([]);
});

test("dependency navigation is keyboard-accessible, preserves list scope, and respects unsaved draft guards", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openDependencies(page, live);
  const fixture = live.core.dependency_reads_project;
  const related = fixture.first_blocker;
  await draftForm(page).getByRole("button", { name: "Edit draft", exact: true }).click();
  await page.getByLabel("Draft title", { exact: true }).fill("Unsaved dependency navigation");
  const open = dependencyPanel(page).getByRole("button", {
    name: `Open related ${related.key}`,
    exact: true,
  });
  page.once("dialog", (dialog) => dialog.dismiss());
  await open.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveValue(
    "Unsaved dependency navigation",
  );
  page.once("dialog", (dialog) => dialog.accept());
  await open.focus();
  await page.keyboard.press("Enter");
  await expect(draftForm(page)).toContainText(related.title);
  const back = page.getByRole("button", { name: "Back to previous task", exact: true });
  const box = await back.boundingBox();
  expect(box).not.toBeNull();
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(375);
  await back.focus();
  await page.keyboard.press("Enter");
  await expect(draftForm(page)).toContainText(fixture.root.title);
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveCount(0);
  expect(
    (await (await commandClient(live, { projectId: fixture.id })).task(fixture.root.id)).title,
  ).toBe(fixture.root.title);
});
