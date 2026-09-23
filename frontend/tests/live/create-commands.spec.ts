import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { TaskCommandReceiptSchema } from "../../src/contracts/task-command";
import { draftForm } from "./draft-helpers";
import { loadProject } from "./task-helpers";
import {
  createButton,
  createClient,
  createPath,
  fillCreate,
  openCreate,
  submitCreate,
} from "./create-command-helpers";

for (const kind of ["delivery", "analysis"] as const) {
  test(`create ${kind} draft pins the explicitly selected older version and survives reload`, async ({
    live,
    page,
  }) => {
    const client = await createClient(live);
    const before = await client.project();
    await openCreate(page, live);
    await expect(createButton(page)).toBeDisabled();
    await expect(page.getByLabel("Task kind", { exact: true })).toHaveValue("");
    await expect(page.getByLabel("Task priority", { exact: true })).toHaveValue("normal");
    const title = `Created ${kind} ${randomUUID()}`;
    await fillCreate(page, live, title, kind);
    await page
      .getByLabel("Draft description", { exact: true })
      .fill("Human-authored draft description");
    if (kind === "analysis")
      await page.getByLabel("Definition of done", { exact: true }).fill("Written comparison");
    const receipt = await submitCreate(page);
    const task = await client.task(receipt.resource.id);
    expect(task).toMatchObject({
      title,
      kind,
      lifecycle: "draft",
      priority: "normal",
      pipeline_version_id: live.core.create_commands_project.older_version_id,
      properties: {},
      definition_of_done: kind === "analysis" ? "Written comparison" : null,
      description: "Human-authored draft description",
    });
    expect((await client.project()).revision).toBe(before.revision + 1);
    expect((await client.project()).execution_gate).toBe("stopped");
    expect((await client.runs()).items).toEqual([]);
    await expect(draftForm(page)).toContainText(title);
    await page.reload();
    await loadProject(page, live.core.create_commands_project.id);
    await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
    await expect(draftForm(page)).toContainText(title);
    await expect(
      draftForm(page).getByRole("button", { name: "Edit draft", exact: true }),
    ).toBeVisible();
  });
}

test("lost create response replays identical bytes and key without a second Task", async ({
  live,
  page,
}) => {
  const client = await createClient(live);
  const before = await client.project();
  await openCreate(page, live);
  const title = `Lost creation ${randomUUID()}`;
  await fillCreate(page, live, title);
  const attempts: { body: string | null; key: string | undefined }[] = [];
  let applied: unknown;
  let replayed: unknown;
  let failed = false;
  await page.route(`${live.origin}${createPath}`, async (route) => {
    try {
      attempts.push({
        body: route.request().postData(),
        key: route.request().headers()["idempotency-key"],
      });
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) throw new Error("Expected real receipt");
      const receipt: unknown = await response.json();
      if (attempts.length === 1) {
        applied = receipt;
        await route.abort("failed");
      } else {
        replayed = receipt;
        await route.fulfill({ response });
      }
    } catch {
      failed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, async () => {
    await createButton(page).click();
    await expect(
      page.getByRole("button", { name: "Retry same creation", exact: true }),
    ).toBeVisible();
  });
  await expect(page.getByLabel("Draft title", { exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Retry same creation", exact: true }).click();
  await expect(draftForm(page)).toContainText(title);
  expect(failed).toBe(false);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  expect(attempts[0]?.key).toBeTruthy();
  expect(replayed).toEqual({ ...(applied as object), status: "replayed" });
  expect((await client.tasks()).items.filter((task) => task.title === title)).toHaveLength(1);
  expect((await client.project()).revision).toBe(before.revision + 1);
  await page.unrouteAll({ behavior: "wait" });
});

test("create conflict preserves every field until explicit baseline refresh and save", async ({
  live,
  page,
}) => {
  const client = await createClient(live);
  await openCreate(page, live);
  const title = `Conflict intent ${randomUUID()}`;
  await fillCreate(page, live, title, "analysis");
  await page.getByLabel("Draft description", { exact: true }).fill("Keep description");
  await page.getByLabel("Definition of done", { exact: true }).fill("Keep DoD");
  await page.getByLabel("Task priority", { exact: true }).selectOption("low");
  expect((await client.post(await client.baseline(`Concurrent ${randomUUID()}`))).status).toBe(200);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await createButton(page).click();
      await expect(
        page.getByRole("button", { name: "Refresh creation baseline", exact: true }),
      ).toBeVisible();
    },
  );
  await expect(page.getByLabel("Draft title", { exact: true })).toHaveValue(title);
  await expect(page.getByLabel("Task kind", { exact: true })).toHaveValue("analysis");
  await expect(page.getByLabel("Task priority", { exact: true })).toHaveValue("low");
  await expect(page.getByLabel("Draft description", { exact: true })).toHaveValue(
    "Keep description",
  );
  await expect(page.getByLabel("Definition of done", { exact: true })).toHaveValue("Keep DoD");
  await page.getByRole("button", { name: "Refresh creation baseline", exact: true }).click();
  await expect(createButton(page)).toBeEnabled();
  expect((await client.tasks()).items.filter((task) => task.title === title)).toHaveLength(0);
  const receipt = await submitCreate(page);
  expect(await client.task(receipt.resource.id)).toMatchObject({
    title,
    priority: "low",
    kind: "analysis",
    description: "Keep description",
    definition_of_done: "Keep DoD",
    pipeline_version_id: live.core.create_commands_project.older_version_id,
  });
});

test("create Core rejects foreign, deleted, incompatible versions and unknown priority without effects", async ({
  live,
}) => {
  const client = await createClient(live);
  const request = await client.baseline(`Rejected ${randomUUID()}`);
  const before = await client.project();
  const fixture = live.core.create_commands_project;
  for (const [patch, status] of [
    [{ pipeline_version_id: fixture.foreign_version_id }, 400],
    [{ pipeline_version_id: fixture.deleted_version_id }, 400],
    [{ pipeline_version_id: fixture.delivery_only_version_id, kind: "analysis" }, 400],
    [{ priority: "missing_level" }, 422],
  ] as const) {
    expect(
      (await client.post({ ...request, payload: { ...request.payload, ...patch } })).status,
    ).toBe(status);
  }
  expect(await client.project()).toEqual(before);
  expect(
    (await client.tasks()).items.filter((task) => task.title === request.payload.title),
  ).toHaveLength(0);
});

test("create gateway rejects hostile headers and fields; key reuse cannot change payload", async ({
  live,
}) => {
  const client = await createClient(live);
  const request = await client.baseline(`Boundary ${randomUUID()}`);
  const before = await client.project();
  for (const headers of [{ Origin: "https://untrusted.invalid" }, { Host: "untrusted.invalid" }])
    expect((await client.post(request, randomUUID(), headers)).status).toBe(403);
  for (const patch of [
    { task_id: live.core.tasks.draft_id },
    { lifecycle: "ready" },
    { pipeline_id: live.core.create_commands_project.pipeline_id },
    { properties: { risk: { type: "text" } } },
  ])
    expect(
      (await client.post({ ...request, payload: { ...request.payload, ...patch } })).status,
    ).toBe(400);
  expect(await client.project()).toEqual(before);
  const key = randomUUID();
  const response = await client.post(request, key);
  expect(response.status).toBe(200);
  const receipt = TaskCommandReceiptSchema.parse(await response.json());
  const replay = await client.post(request, key);
  expect(TaskCommandReceiptSchema.parse(await replay.json())).toEqual({
    ...receipt,
    status: "replayed",
  });
  expect(
    (await client.post({ ...request, payload: { ...request.payload, title: "Different" } }, key))
      .status,
  ).toBe(409);
  expect((await client.project()).revision).toBe(before.revision + 1);
});
