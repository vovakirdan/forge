import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { holdRealResponse, loadProject } from "./task-helpers";
import {
  createButton,
  createClient,
  createForm,
  createPath,
  fillCreate,
  openCreate,
} from "./create-command-helpers";

test("conflict refresh retains a now-deleted selected Pipeline but blocks creation", async ({
  live,
  page,
}) => {
  const client = await createClient(live);
  await openCreate(page, live);
  const title = `Deleted on refresh ${randomUUID()}`;
  await fillCreate(page, live, title);
  expect(
    (await client.post(await client.baseline(`Concurrent Pipeline change ${randomUUID()}`))).status,
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
  const fixture = live.core.create_commands_project;
  const url = `${live.origin}/api/projects/${fixture.id}/pipelines/${fixture.older_version_id}`;
  let failed = false;
  await page.route(url, async (route) => {
    try {
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real Pipeline");
      const detail = await response.json();
      await route.fulfill({ response, json: { ...detail, deleted_at: "2026-09-18T00:00:00Z" } });
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await page.getByRole("button", { name: "Refresh creation baseline", exact: true }).click();
  await expect(createForm(page)).toContainText("Selected Pipeline is deleted");
  await expect(createForm(page)).toContainText(fixture.older_version_id);
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveValue(title);
  await expect(createButton(page)).toBeDisabled();
  expect(failed).toBe(false);
  expect((await client.tasks()).items.filter((task) => task.title === title)).toHaveLength(0);
  await page.unroute(url);
});

test("creation browses real Pipeline pages without implicit selection or eager loading", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.pipelines_project.id);
  const requests: string[] = [];
  let posts = 0;
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === `/api/projects/${live.core.pipelines_project.id}/pipelines`)
      requests.push(url.search);
    if (url.pathname === createPath) posts += 1;
  });
  await page.getByRole("button", { name: "Create draft", exact: true }).click();
  const choices = createForm(page).getByRole("region", {
    name: "Pipeline version choices",
    exact: true,
  });
  await expect(choices.getByRole("button", { name: /^Select Pipeline version / })).toHaveCount(20);
  expect(requests).toHaveLength(1);
  await expect(createForm(page)).toContainText("None selected");
  await page.getByRole("button", { name: "Next Pipeline page", exact: true }).click();
  await expect(choices.getByRole("button", { name: /^Select Pipeline version / })).toHaveCount(
    live.core.pipelines_project.total - 20,
  );
  expect(requests).toHaveLength(2);
  await expect(createButton(page)).toBeDisabled();
  await expect(createForm(page)).toContainText("None selected");
  expect(posts).toBe(0);
});

test("project without Pipelines cannot create a draft or invent a Pipeline", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Create draft", exact: true }).click();
  await page.getByLabel("Draft title", { exact: true }).fill("No pipeline available");
  await page.getByLabel("Task kind", { exact: true }).selectOption("delivery");
  await expect(createForm(page)).toContainText("No Pipeline versions on this page.");
  await expect(createButton(page)).toBeDisabled();
});

test("fresh selected version rejects incompatible kind and synthetic deleted detail", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  await page.getByLabel("Draft title", { exact: true }).fill(`Pipeline checks ${randomUUID()}`);
  await page.getByLabel("Task kind", { exact: true }).selectOption("analysis");
  const fixture = live.core.create_commands_project;
  await page
    .getByRole("button", {
      name: `Select Pipeline version ${fixture.delivery_only_version_id}`,
      exact: true,
    })
    .click();
  await expect(createForm(page)).toContainText("does not support this Task kind");
  await expect(createButton(page)).toBeDisabled();
  await expect(
    page.getByRole("button", {
      name: `Select Pipeline version ${fixture.deleted_version_id}`,
      exact: true,
    }),
  ).toBeDisabled();
  let failed = false;
  const url = `${live.origin}/api/projects/${fixture.id}/pipelines/${fixture.older_version_id}`;
  await page.route(url, async (route) => {
    try {
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real detail");
      const detail = await response.json();
      await route.fulfill({ response, json: { ...detail, deleted_at: "2026-09-18T00:00:00Z" } });
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await page
    .getByRole("button", {
      name: `Select Pipeline version ${fixture.older_version_id}`,
      exact: true,
    })
    .click();
  await expect(createForm(page)).toContainText("Selected Pipeline is deleted");
  await expect(createButton(page)).toBeDisabled();
  expect(failed).toBe(false);
  await page.unroute(url);
});

test("Project switch cancels selected Pipeline detail and cannot restore creation", async ({
  live,
  page,
}) => {
  await openCreate(page, live);
  const fixture = live.core.create_commands_project;
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${fixture.id}/pipelines/${fixture.older_version_id}`,
  );
  try {
    await page
      .getByRole("button", {
        name: `Select Pipeline version ${fixture.older_version_id}`,
        exact: true,
      })
      .click();
    await held.ready;
    held.assertCaptured();
    page.once("dialog", (dialog) => dialog.accept());
    await loadProject(page, live.core.empty_project.id);
    await held.releaseAfterCancellation();
    await expect(createForm(page)).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
});
