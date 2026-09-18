import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { draftForm } from "./draft-helpers";
import { fact } from "./pipeline-helpers";
import { loadProject } from "./task-helpers";
import {
  openPriority,
  priorityClient,
  priorityCommandPath,
  priorityDraft,
  priorityEditor,
} from "./priority-command-helpers";

test("priority saves through real Core, reloads, and unchanged selection sends nothing", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 0);
  const client = await priorityClient(live);
  const before = await client.task(draft.id);
  const project = await client.project();
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(priorityCommandPath)) posts += 1;
  });
  const save = priorityEditor(page).getByRole("button", { name: "Save priority", exact: true });
  await expect(save).toBeDisabled();
  expect(posts).toBe(0);
  await expect(page.getByRole("button", { name: "Edit draft", exact: true })).toHaveCount(0);
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  await save.click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  await expect(priorityEditor(page)).toContainText("Current saved priority: High (high)");
  const after = await client.task(draft.id);
  expect(after.priority).toBe("high");
  expect(after.revision).toBe(before.revision + 1);
  for (const field of [
    "title",
    "description",
    "kind",
    "lifecycle",
    "pipeline_version_id",
    "current_stage_id",
    "definition_of_done",
    "properties",
    "artifacts",
    "wait_conditions",
  ] as const)
    expect(after[field]).toEqual(before[field]);
  expect((await client.project()).revision).toBe(project.revision + 1);
  expect((await client.project()).execution_gate).toBe("stopped");
  expect((await client.runs()).items).toEqual([]);
  await fact(draftForm(page), "Priority", "High");
  expect(posts).toBe(1);
  await page.reload();
  await loadProject(page, live.core.priority_commands_project.id);
  await page.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await fact(draftForm(page), "Priority", "High");
  await page.getByRole("button", { name: "Change priority", exact: true }).click();
  await expect(page.getByLabel("Task priority", { exact: true })).toHaveValue("high");
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toBeDisabled();
});

test("priority lost applied response replays frozen bytes and key with one effect", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 1);
  const client = await priorityClient(live);
  const before = await client.task(draft.id);
  const attempts: { body: string | null; key: string | undefined }[] = [];
  let applied: unknown;
  let replayed: unknown;
  let failed = false;
  await page.route(`${live.origin}${priorityCommandPath}`, async (route) => {
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
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, async () => {
    await page.getByRole("button", { name: "Save priority", exact: true }).click();
    await expect(page.getByRole("button", { name: "Retry same save", exact: true })).toBeVisible();
  });
  await expect(page.getByLabel("Task priority", { exact: true })).toBeDisabled();
  await expect(priorityEditor(page)).toContainText("Last loaded priority: Normal (normal)");
  await expect(priorityEditor(page)).not.toContainText("Current saved priority:");
  expect((await client.task(draft.id)).revision).toBe(before.revision + 1);
  await page.getByRole("button", { name: "Retry same save", exact: true }).click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  expect(failed).toBe(false);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  expect(attempts[0]?.key).toBeTruthy();
  expect(replayed).toEqual({ ...(applied as object), status: "replayed" });
  expect((await client.task(draft.id)).revision).toBe(before.revision + 1);
  await page.unrouteAll({ behavior: "wait" });
});

test("priority conflict in two tabs retains choice and requires explicit refresh then save", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 2);
  const second = await page.context().newPage();
  await openPriority(second, live, 2);
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  await second.getByLabel("Task priority", { exact: true }).selectOption("low");
  await expect(second.getByLabel("Task priority", { exact: true })).toHaveValue("low");
  await page.getByRole("button", { name: "Save priority", exact: true }).click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await second.getByRole("button", { name: "Save priority", exact: true }).click();
      await expect(
        second.getByRole("button", { name: "Refresh priority baseline", exact: true }),
      ).toBeVisible();
    },
  );
  await expect(second.getByLabel("Task priority", { exact: true })).toHaveValue("low");
  const client = await priorityClient(live);
  expect((await client.task(draft.id)).priority).toBe("high");
  await second.getByRole("button", { name: "Refresh priority baseline", exact: true }).click();
  await expect(second.getByRole("button", { name: "Save priority", exact: true })).toBeEnabled();
  expect((await client.task(draft.id)).priority).toBe("high");
  await second.getByRole("button", { name: "Save priority", exact: true }).click();
  await expect(priorityEditor(second)).toContainText("Authoritative data refreshed.");
  expect((await client.task(draft.id)).priority).toBe("low");
});

test("draft edit conflicts with priority baseline without losing either intent", async ({
  live,
  page,
}) => {
  const draft = await openPriority(page, live, 3);
  const other = await page.context().newPage();
  await live.login(other);
  await loadProject(other, live.core.priority_commands_project.id);
  await other.getByRole("button", { name: `Open ${draft.key}`, exact: true }).click();
  await other.getByRole("button", { name: "Edit draft", exact: true }).click();
  await other
    .getByLabel("Draft description", { exact: true })
    .fill("Independent draft description");
  await expect(other.getByRole("button", { name: "Change priority", exact: true })).toHaveCount(0);
  await page.getByLabel("Task priority", { exact: true }).selectOption("high");
  await draftForm(other).getByRole("button", { name: "Save", exact: true }).click();
  await expect(draftForm(other)).toContainText("Authoritative data refreshed.");
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await page.getByRole("button", { name: "Save priority", exact: true }).click();
      await expect(
        page.getByRole("button", { name: "Refresh priority baseline", exact: true }),
      ).toBeVisible();
    },
  );
  await page.getByRole("button", { name: "Refresh priority baseline", exact: true }).click();
  await expect(page.getByRole("button", { name: "Save priority", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Save priority", exact: true }).click();
  await expect(priorityEditor(page)).toContainText("Authoritative data refreshed.");
  expect(await (await priorityClient(live)).task(draft.id)).toMatchObject({
    priority: "high",
    description: "Independent draft description",
  });
});

test("priority Core rejects stale task, unknown level, terminal and foreign Task", async ({
  live,
  page,
}) => {
  const client = await priorityClient(live);
  const draft = priorityDraft(live, 4);
  const first = await client.priorityBaseline(draft.id, "high");
  const key = randomUUID();
  expect((await client.post(first, key)).status).toBe(200);
  const current = await client.priorityBaseline(draft.id, "low");
  expect(
    (
      await client.post({
        ...current,
        payload: {
          ...current.payload,
          expected_task_revision: first.payload.expected_task_revision,
        },
      })
    ).status,
  ).toBe(400);
  expect(
    (await client.post({ ...first, payload: { ...first.payload, priority: "low" } }, key)).status,
  ).toBe(409);
  expect(
    (
      await client.post({
        ...current,
        payload: { ...current.payload, task_id: live.core.second_project.task_id },
      })
    ).status,
  ).toBe(400);
  expect((await client.post(await client.priorityBaseline(draft.id, "missing_level"))).status).toBe(
    422,
  );
  expect(
    (
      await client.post(
        await client.priorityBaseline(live.core.priority_commands_project.cancelled_id, "high"),
      )
    ).status,
  ).toBe(422);
  expect((await client.task(draft.id)).revision).toBe(first.payload.expected_task_revision + 1);
  await live.login(page);
  await loadProject(page, live.core.priority_commands_project.id);
  const cancelled = await client.task(live.core.priority_commands_project.cancelled_id);
  await page.getByRole("button", { name: `Open ${cancelled.key}`, exact: true }).click();
  await expect(page.getByRole("button", { name: "Change priority", exact: true })).toHaveCount(0);
});

test("priority gateway rejects hostile headers and extra fields without writing", async ({
  live,
}) => {
  const client = await priorityClient(live);
  const draft = priorityDraft(live, 5);
  const body = await client.priorityBaseline(draft.id, "high");
  for (const headers of [
    { Origin: "null" },
    { Origin: "https://evil.invalid" },
    { Host: "evil.invalid" },
    { Authorization: "Bearer invalid" },
  ])
    expect([401, 403]).toContain((await client.post(body, randomUUID(), headers)).status);
  for (const invalid of [
    { ...body, actor: "human" },
    { ...body, payload: { ...body.payload, lifecycle: "done" } },
    { ...body, payload: { ...body.payload, priority: null } },
    { ...body, payload: { ...body.payload, priority: "../high" } },
  ])
    expect((await client.post(invalid)).status).toBe(400);
  expect((await client.post(body, "x".repeat(129))).status).toBe(400);
  expect((await client.task(draft.id)).revision).toBe(body.payload.expected_task_revision);
  expect((await client.project()).revision).toBe(body.expected_revision);
});
