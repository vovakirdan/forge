import { test, expect } from "./fixtures";
import { draftForm } from "./draft-helpers";
import { holdRealResponse, loadProject } from "./task-helpers";
import {
  cancellationClient,
  cancellationDraft,
  cancellationPanel,
  cancellationPath,
  confirmCancellation,
  consent,
  openCancellation,
  reason,
  selectCancellation,
} from "./cancellation-helpers";

for (const fault of ["lost response", "malformed receipt"] as const) {
  test(`cancellation ${fault} after real commit retries exact bytes and key`, async ({
    live,
    page,
  }) => {
    const task = await openCancellation(
      page,
      live,
      cancellationDraft(live, fault === "lost response" ? 4 : 5).id,
    );
    const client = await cancellationClient(live);
    const attempts: { body: string | null; key: string | undefined }[] = [];
    let applied: unknown;
    let replayed: unknown;
    let failed = false;
    await page.route(`${live.origin}${cancellationPath}`, async (route) => {
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
    await selectCancellation(page);
    const submit = async () => {
      await confirmCancellation(page).click();
      await expect(
        page.getByRole("button", { name: "Retry same cancellation", exact: true }),
      ).toBeVisible();
    };
    if (fault === "lost response")
      await live.allowConsoleErrors(/^Failed to load resource: net::ERR_FAILED$/, submit);
    else await submit();
    await expect(cancellationPanel(page)).not.toContainText("Cancellation accepted.");
    await expect(consent(page)).toBeDisabled();
    await expect(reason(page)).toBeDisabled();
    expect((await client.task(task.id)).lifecycle).toBe("cancelled");
    const revision = (await client.project()).revision;
    await page.getByRole("button", { name: "Retry same cancellation", exact: true }).click();
    await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
    expect(failed).toBe(false);
    expect(attempts).toHaveLength(2);
    expect(attempts[1]).toEqual(attempts[0]);
    expect(attempts[0]?.key).toBeTruthy();
    expect(replayed).toEqual({ ...(applied as object), status: "replayed" });
    expect((await client.project()).revision).toBe(revision);
    expect((await client.task(task.id)).revision).toBe(task.revision + 1);
    await page.unrouteAll({ behavior: "wait" });
  });
}

test("accepted cancellation survives failed readback; refresh retries only GET", async ({
  live,
  page,
}) => {
  const task = await openCancellation(page, live, cancellationDraft(live, 6).id);
  await selectCancellation(page);
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(cancellationPath)) posts += 1;
  });
  const url = `${live.origin}/api/projects/${live.core.cancellation_commands_project.id}/tasks/${task.id}`;
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await confirmCancellation(page).click();
      await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
      await expect(
        page.getByRole("button", { name: "Refresh cancelled data", exact: true }),
      ).toBeVisible();
    },
  );
  expect(posts).toBe(1);
  await page.unroute(url);
  await page.getByRole("button", { name: "Refresh cancelled data", exact: true }).click();
  await expect(cancellationPanel(page)).toContainText("Authoritative data refreshed.");
  expect(posts).toBe(1);
  expect((await (await cancellationClient(live)).task(task.id)).lifecycle).toBe("cancelled");
});

for (const destination of ["logout", "project"] as const) {
  test(`cancellation discards late response after ${destination} without undoing Core`, async ({
    live,
    page,
  }) => {
    const task = await openCancellation(
      page,
      live,
      cancellationDraft(live, destination === "logout" ? 7 : 8).id,
    );
    const client = await cancellationClient(live);
    await selectCancellation(page);
    const held = await holdRealResponse(page, `${live.origin}${cancellationPath}`);
    try {
      await confirmCancellation(page).click();
      await held.ready;
      held.assertCaptured();
      page.once("dialog", (dialog) => dialog.dismiss());
      await page.getByRole("button", { name: "Runs", exact: true }).click();
      await expect(cancellationPanel(page)).toBeVisible();
      page.once("dialog", (dialog) => dialog.accept());
      if (destination === "logout") {
        await page.getByRole("button", { name: "Log out", exact: true }).click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      } else await loadProject(page, live.core.empty_project.id);
      await held.releaseAfterCancellation();
      await expect(cancellationPanel(page)).toHaveCount(0);
      await expect(draftForm(page)).toHaveCount(0);
      expect((await client.task(task.id)).lifecycle).toBe("cancelled");
    } finally {
      await held.cleanup();
    }
  });
}

test("unavailable cancellation catalog blocks submission and can be explicitly retried", async ({
  live,
  page,
}) => {
  const task = cancellationDraft(live, 9);
  const pattern = `${live.origin}/api/projects/${live.core.cancellation_commands_project.id}/cancellation-reasons`;
  await page.route(pattern, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable" } }),
  );
  await live.login(page);
  await loadProject(page, live.core.cancellation_commands_project.id);
  await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await draftForm(page).getByRole("button", { name: "Cancel task", exact: true }).click();
      await expect(cancellationPanel(page).getByRole("alert")).toBeVisible();
    },
  );
  await expect(confirmCancellation(page)).toHaveCount(0);
  await page.unroute(pattern);
  await cancellationPanel(page)
    .getByRole("button", { name: "Retry cancellation baseline", exact: true })
    .click();
  await expect(reason(page)).toBeEnabled();
  await expect(confirmCancellation(page)).toBeDisabled();
  expect((await (await cancellationClient(live)).task(task.id)).lifecycle).toBe("draft");
});

test("retired catalog entries are not selectable; saved reason remains readable without catalog", async ({
  live,
  page,
}) => {
  const url = `${live.origin}/api/projects/${live.core.cancellation_commands_project.id}/cancellation-reasons`;
  // Presentation-only retired entry; actual Core settings remain unchanged.
  await page.route(url, async (route) => {
    const response = await route.fetch();
    const catalog = await response.json();
    await route.fulfill({
      response,
      json: {
        ...catalog,
        reasons: [
          ...catalog.reasons,
          { id: "retired_reason", display_name: "Old reason", retired: true },
        ],
      },
    });
  });
  const task = await openCancellation(page, live, cancellationDraft(live, 10).id);
  await expect(reason(page).getByRole("option", { name: /Old reason/ })).toHaveCount(0);
  await selectCancellation(page);
  await confirmCancellation(page).click();
  await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
  await expect(cancellationPanel(page)).toContainText("Authoritative data refreshed.");
  await page.getByRole("button", { name: "Close cancellation", exact: true }).click();
  expect((await (await cancellationClient(live)).task(task.id)).cancellation?.reason_id).toBe(
    "unspecified",
  );
  await page.unroute(url);
  await page.route(url, (route) => route.fulfill({ status: 503, json: { error: "Unavailable" } }));
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await page.reload();
      await loadProject(page, live.core.cancellation_commands_project.id);
      await page.getByRole("button", { name: `Open ${task.key}`, exact: true }).click();
      await expect(draftForm(page)).toContainText("unspecified");
    },
  );
});
