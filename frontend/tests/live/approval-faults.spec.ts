import { test, expect } from "./fixtures";
import { draftForm } from "./draft-helpers";
import { holdRealResponse, loadProject } from "./task-helpers";
import {
  approvalClient,
  approvalDraft,
  approvalPanel,
  approvalPath,
  confirmApproval,
  consent,
  openApproval,
} from "./approval-helpers";

for (const fault of ["lost response", "malformed receipt"] as const) {
  test(`approval ${fault} after real commit retries identical bytes and key with one effect`, async ({
    live,
    page,
  }) => {
    const task = await openApproval(
      page,
      live,
      approvalDraft(live, fault === "lost response" ? 4 : 5).id,
    );
    const client = await approvalClient(live);
    const before = await client.project();
    const attempts: { body: string | null; key: string | undefined }[] = [];
    let applied: unknown;
    let replayed: unknown;
    let failed = false;
    await page.route(`${live.origin}${approvalPath}`, async (route) => {
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
          if (fault === "lost response") await route.abort("failed");
          else await route.fulfill({ status: 200, json: { status: "applied" } });
        } else {
          replayed = receipt;
          await route.fulfill({ response });
        }
      } catch {
        failed = true;
        await route.abort().catch(() => undefined);
      }
    });
    await consent(page).check();
    const submit = async () => {
      await confirmApproval(page).click();
      await expect(
        page.getByRole("button", { name: "Retry same approval", exact: true }),
      ).toBeVisible();
    };
    if (fault === "lost response")
      await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, submit);
    else await submit();
    await expect(approvalPanel(page)).not.toContainText("Approved.");
    await expect(consent(page)).toBeDisabled();
    expect((await client.task(task.id)).lifecycle).toBe("ready");
    await page.getByRole("button", { name: "Retry same approval", exact: true }).click();
    await expect(approvalPanel(page)).toContainText("Approved.");
    expect(failed).toBe(false);
    expect(attempts).toHaveLength(2);
    expect(attempts[1]).toEqual(attempts[0]);
    expect(attempts[0]?.key).toBeTruthy();
    expect(replayed).toEqual({ ...(applied as object), status: "replayed" });
    expect((await client.project()).revision).toBe(before.revision + 1);
    expect((await client.task(task.id)).revision).toBe(task.revision + 1);
    await page.unrouteAll({ behavior: "wait" });
  });
}

test("confirmed approval survives failed readback and retries only reads", async ({
  live,
  page,
}) => {
  const task = await openApproval(page, live, approvalDraft(live, 6).id);
  await consent(page).check();
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(approvalPath)) posts += 1;
  });
  const url = `${live.origin}/api/projects/${live.core.approval_commands_project.id}/tasks/${task.id}`;
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await confirmApproval(page).click();
      await expect(approvalPanel(page)).toContainText("Approved.");
      await expect(
        page.getByRole("button", { name: "Refresh approved data", exact: true }),
      ).toBeVisible();
    },
  );
  expect(posts).toBe(1);
  await page.unroute(url);
  await page.getByRole("button", { name: "Refresh approved data", exact: true }).click();
  await expect(approvalPanel(page)).toContainText("Authoritative data refreshed.");
  expect(posts).toBe(1);
  expect((await (await approvalClient(live)).task(task.id)).lifecycle).toBe("ready");
});

for (const destination of ["logout", "project"] as const) {
  test(`approval discards late response after ${destination} without undoing Core`, async ({
    live,
    page,
  }) => {
    const task = await openApproval(
      page,
      live,
      approvalDraft(live, destination === "logout" ? 7 : 8).id,
    );
    const client = await approvalClient(live);
    await consent(page).check();
    const held = await holdRealResponse(page, `${live.origin}${approvalPath}`);
    try {
      await confirmApproval(page).click();
      await held.ready;
      held.assertCaptured();
      page.once("dialog", (dialog) => dialog.dismiss());
      await page.getByRole("button", { name: "Runs", exact: true }).click();
      await expect(approvalPanel(page)).toBeVisible();
      page.once("dialog", (dialog) => dialog.accept());
      if (destination === "logout") {
        await page.getByRole("button", { name: "Log out", exact: true }).click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      } else await loadProject(page, live.core.empty_project.id);
      await held.releaseAfterCancellation();
      await expect(approvalPanel(page)).toHaveCount(0);
      await expect(draftForm(page)).toHaveCount(0);
      expect((await client.task(task.id)).lifecycle).toBe("ready");
    } finally {
      await held.cleanup();
    }
  });
}

test("unavailable pinned Pipeline blocks confirmation and supports explicit baseline retry", async ({
  live,
  page,
}) => {
  const task = approvalDraft(live, 9);
  const pattern = `${live.origin}/api/projects/${live.core.approval_commands_project.id}/pipelines/${live.core.approval_commands_project.pipeline_version_id}`;
  await page.route(pattern, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable" } }),
  );
  await live.login(page);
  await loadProject(page, live.core.approval_commands_project.id);
  await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await draftForm(page).getByRole("button", { name: "Approve draft", exact: true }).click();
      await expect(approvalPanel(page).getByRole("alert")).toBeVisible();
    },
  );
  await expect(confirmApproval(page)).toHaveCount(0);
  await page.unroute(pattern);
  await approvalPanel(page)
    .getByRole("button", { name: "Retry approval baseline", exact: true })
    .click();
  await expect(consent(page)).toBeEnabled();
  await expect(confirmApproval(page)).toBeDisabled();
  expect((await (await approvalClient(live)).task(task.id)).lifecycle).toBe("draft");
});
