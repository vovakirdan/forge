import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { commandClient, commandPath, draftForm, fixtureDraft, openDraft } from "./draft-helpers";

test("draft text saves through real Core without changing other fields or creating Runs", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 0);
  const client = await commandClient(live);
  const before = await client.task(draft.id);
  const baseline = await client.project();
  const title = "  План 🛠 <img src=x onerror=window.forgeInjected=true>  ";
  const description = "Первая строка.\n<script>window.forgeInjected=true</script>";
  await page.getByLabel("Draft title", { exact: true }).fill(title);
  await page.getByLabel("Draft description", { exact: true }).fill(description);
  const response = page.waitForResponse(`${live.origin}${commandPath}`);
  await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
  expect((await response).status()).toBe(200);
  await expect(draftForm(page)).toContainText("Saved.");
  await expect(page.getByRole("region", { name: "Tasks", exact: true })).toContainText(
    title.trim(),
  );
  const after = await client.task(draft.id);
  expect(after.title).toBe(title);
  expect(after.description).toBe(description);
  expect(after.revision).toBe(before.revision + 1);
  expect(after.lifecycle).toBe("draft");
  for (const field of [
    "kind",
    "priority",
    "pipeline_version_id",
    "current_stage_id",
    "definition_of_done",
    "properties",
    "artifacts",
    "wait_conditions",
  ] as const)
    expect(after[field]).toEqual(before[field]);
  expect((await client.project()).revision).toBe(baseline.revision + 1);
  expect((await client.project()).execution_gate).toBe("stopped");
  expect((await client.runs()).items).toHaveLength(0);
  expect(await page.evaluate(() => "forgeInjected" in window)).toBe(false);
});

test("lost real applied response retries frozen bytes and key with one effect", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 1);
  const client = await commandClient(live);
  const before = await client.task(draft.id);
  const attempts: { body: string | null; key: string | undefined }[] = [];
  let applied: unknown;
  let replayed: unknown;
  let routeFailed = false;
  await page.route(`${live.origin}${commandPath}`, async (route) => {
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
      routeFailed = true;
      await route.abort().catch(() => undefined);
    }
  });
  await page.getByLabel("Draft title", { exact: true }).fill("Exactly one accepted edit");
  await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, async () => {
    await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByRole("button", { name: "Retry same save", exact: true })).toBeVisible();
  });
  expect(attempts).toHaveLength(1);
  expect((await client.task(draft.id)).revision).toBe(before.revision + 1);
  await page.getByRole("button", { name: "Retry same save", exact: true }).click();
  await expect(draftForm(page)).toContainText("Saved.");
  expect(routeFailed).toBe(false);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  expect(attempts[0]?.key).toBeTruthy();
  expect(applied).toMatchObject({ status: "applied", resource: { kind: "task", id: draft.id } });
  expect(replayed).toEqual({ ...(applied as object), status: "replayed" });
  expect((await client.task(draft.id)).revision).toBe(before.revision + 1);
  await page.unrouteAll({ behavior: "wait" });
});

test("two tabs conflict explicitly and refresh preserves only locally changed fields", async ({
  live,
  page,
}) => {
  const draft = await openDraft(page, live, 2);
  const second = await page.context().newPage();
  await openDraft(second, live, 2);
  await page.getByLabel("Draft description", { exact: true }).fill("First tab description");
  await second.getByLabel("Draft title", { exact: true }).fill("Second tab title");
  await draftForm(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(draftForm(page)).toContainText("Saved.");
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await draftForm(second).getByRole("button", { name: "Save", exact: true }).click();
      await expect(
        second.getByRole("button", { name: "Refresh edit baseline", exact: true }),
      ).toBeVisible();
    },
  );
  await expect(second.getByLabel("Draft title", { exact: true })).toHaveValue("Second tab title");
  const client = await commandClient(live);
  expect((await client.task(draft.id)).title).not.toBe("Second tab title");
  await second.getByRole("button", { name: "Refresh edit baseline", exact: true }).click();
  await expect(second.getByLabel("Draft description", { exact: true })).toHaveValue(
    "First tab description",
  );
  await draftForm(second).getByRole("button", { name: "Save", exact: true }).click();
  await expect(draftForm(second)).toContainText("Saved.");
  expect(await client.task(draft.id)).toMatchObject({
    title: "Second tab title",
    description: "First tab description",
  });
});

test("real Core refusals preserve stale Task semantics and project boundaries", async ({
  live,
}) => {
  const client = await commandClient(live);
  const draft = fixtureDraft(live, 3);
  const initial = await client.baseline(draft.id, { title: "Fresh accepted text" });
  const key = randomUUID();
  expect((await client.post(initial, key)).status).toBe(200);
  const current = await client.baseline(draft.id, { title: "Must not be accepted" });
  const staleTask = {
    ...current,
    payload: { ...current.payload, expected_task_revision: initial.payload.expected_task_revision },
  };
  let response = await client.post(staleTask);
  expect(response.status).toBe(400);
  expect(await response.json()).toMatchObject({ code: "invalid_request" });
  response = await client.post(
    {
      ...initial,
      payload: { ...initial.payload, patch: { title: "Different payload, same key" } },
    },
    key,
  );
  expect(response.status).toBe(409);
  expect(await response.json()).toMatchObject({ code: "idempotency_conflict" });
  response = await client.post({
    ...current,
    payload: { ...current.payload, task_id: live.core.second_project.task_id },
  });
  expect(response.status).toBe(400);
  expect(await response.json()).toMatchObject({ code: "invalid_request" });
  response = await client.post(
    await client.baseline(live.core.commands_project.cancelled_id, {
      title: "Cannot edit cancelled",
    }),
  );
  expect(response.status).toBe(422);
  expect(await response.json()).toMatchObject({ code: "validation_failed" });
  response = await client.post(await client.baseline(draft.id, { title: "  \n\t" }));
  expect(response.status).toBe(422);
  expect((await client.task(draft.id)).title).toBe("Fresh accepted text");
  expect((await client.task(draft.id)).revision).toBe(initial.payload.expected_task_revision + 1);
});

test("rejected mutation boundary requests do not change canonical Task", async ({ live }) => {
  const client = await commandClient(live);
  const draft = fixtureDraft(live, 4);
  const body = await client.baseline(draft.id, { title: "Must never persist" });
  for (const headers of [
    { Origin: "null" },
    { Origin: "https://evil.invalid" },
    { Host: "evil.invalid" },
    { Authorization: "Bearer invalid" },
  ]) {
    const response = await client.post(body, randomUUID(), headers);
    expect([401, 403]).toContain(response.status);
  }
  for (const invalid of [
    { ...body, actor: "human" },
    { ...body, payload: { ...body.payload, patch: {} } },
    { ...body, payload: { ...body.payload, patch: { priority: "urgent" } } },
    { ...body, payload: { ...body.payload, patch: { title: null } } },
  ])
    expect((await client.post(invalid)).status).toBe(400);
  expect((await client.post(body, "x".repeat(129))).status).toBe(400);
  expect(
    (
      await client.post({
        ...body,
        payload: { ...body.payload, patch: { description: "x".repeat(512 * 1024) } },
      })
    ).status,
  ).toBe(413);
  expect((await client.task(draft.id)).revision).toBe(body.payload.expected_task_revision);
  expect((await client.project()).revision).toBe(body.expected_revision);
});
