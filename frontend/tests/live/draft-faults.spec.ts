import { test, expect } from "./fixtures";
import { commandClient, commandPath, draftForm, openDraft } from "./draft-helpers";
import { holdRealResponse, loadProject } from "./task-helpers";

test("confirmed save survives failed follow-up reads and refresh never resubmits", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 5);
  const taskUrl = `${live.origin}/api/projects/${live.core.commands_project.id}/tasks/${draft.id}`;
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(commandPath)) posts += 1;
  });
  await page.route(taskUrl, (route) =>
    route.fulfill({ status: 503, json: { error: "service unavailable" } }),
  );
  await page
    .getByLabel("Draft description", { exact: true })
    .fill("Persisted even when reads are unavailable");
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
      await expect(draftForm(page)).toContainText("The save is confirmed.");
    },
  );
  expect(posts).toBe(1);
  const client = await commandClient(live);
  expect((await client.task(draft.id)).description).toBe(
    "Persisted even when reads are unavailable",
  );
  await page.unroute(taskUrl);
  await page.getByRole("button", { name: "Refresh saved data", exact: true }).click();
  await expect(draftForm(page)).toContainText("Authoritative data refreshed.");
  expect(posts).toBe(1);
});

test("unsaved navigation warns, can be declined, and accepted leave discards the local buffer", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 6);
  const client = await commandClient(live);
  const before = await client.task(draft.id);
  await page.getByLabel("Draft title", { exact: true }).fill("Unsaved private draft text");
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveValue(
    "Unsaved private draft text",
  );
  page.once("dialog", (dialog) => dialog.accept());
  await loadProject(page, live.core.empty_project.id);
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveCount(0);
  await loadProject(page, live.core.commands_project.id);
  await page.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await page.getByRole("button", { name: "Edit draft", exact: true }).click();
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveValue(before.title);
  expect((await client.task(draft.id)).revision).toBe(before.revision);
});

test("logout during an accepted save discards late UI response but does not undo Core", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 7);
  const client = await commandClient(live);
  const before = await client.task(draft.id);
  const held = await holdRealResponse(page, `${live.origin}${commandPath}`);
  try {
    await page.getByLabel("Draft title", { exact: true }).fill("Accepted before local logout");
    await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
    await held.ready;
    held.assertCaptured();
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Log out", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Connect to Forge", exact: true }),
    ).toBeVisible();
    await held.releaseAfterCancellation();
    await expect(draftForm(page)).toHaveCount(0);
    await expect(page.getByText(/^Saved\./)).toHaveCount(0);
    expect(await client.task(draft.id)).toMatchObject({
      title: "Accepted before local logout",
      revision: before.revision + 1,
    });
    await live.login(page);
    await loadProject(page, live.core.commands_project.id);
    await expect(page.getByRole("region", { name: "Tasks", exact: true })).toContainText(
      "Accepted before local logout",
    );
  } finally {
    await held.cleanup();
  }
});

test("Unicode validation blocks invalid edits and permits an empty description", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 8);
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(commandPath)) posts += 1;
  });
  const save = draftForm(page).getByRole("button", { name: "Save", exact: true });
  await page.getByLabel("Draft title", { exact: true }).fill(" \t ");
  await expect(save).toBeDisabled();
  await page.getByLabel("Draft title", { exact: true }).fill("🛠".repeat(241));
  await expect(save).toBeDisabled();
  await page.getByLabel("Draft title", { exact: true }).fill("🛠".repeat(240));
  await page.getByLabel("Draft description", { exact: true }).fill("x".repeat(50_001));
  await expect(save).toBeDisabled();
  expect(posts).toBe(0);
  await page.getByLabel("Draft description", { exact: true }).fill("");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(draftForm(page)).toContainText("Saved.");
  expect(await (await commandClient(live)).task(draft.id)).toMatchObject({
    title: "🛠".repeat(240),
    description: "",
  });
  expect(posts).toBe(1);
});

test("malformed receipt after a real commit is unknown rather than false success", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 9);
  let posts = 0;
  let routeFailed = false;
  await page.route(`${live.origin}${commandPath}`, async (route) => {
    try {
      posts += 1;
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real receipt");
      if (posts === 1)
        await route.fulfill({ status: 200, json: { status: "applied", unexpected: true } });
      else await route.fulfill({ response });
    } catch {
      routeFailed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await page.getByLabel("Draft title", { exact: true }).fill("Committed with untrusted receipt");
  await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("button", { name: "Retry same save", exact: true })).toBeVisible();
  await expect(draftForm(page).getByRole("button", { name: "Save", exact: true })).toBeDisabled();
  await expect(draftForm(page)).not.toContainText("Saved.");
  await page.getByRole("button", { name: "Retry same save", exact: true }).click();
  await expect(draftForm(page)).toContainText("Saved.");
  expect(routeFailed).toBe(false);
  expect(posts).toBe(2);
  expect((await (await commandClient(live)).task(draft.id)).title).toBe(
    "Committed with untrusted receipt",
  );
  await page.unrouteAll({ behavior: "wait" });
});
