import { test, expect } from "./fixtures";
import { commandClient, draftForm } from "./draft-helpers";
import {
  cancellationPanel,
  cancellationPath,
  confirmCancellation,
  selectCancellation,
} from "./cancellation-helpers";
import {
  dependencyPanel,
  dependencyUrl,
  openDependencies,
  relatedButtons,
} from "./dependency-helpers";
import { holdRealResponse, loadProject } from "./task-helpers";

test("accepted cancellation refreshes dependency conditions even when Task readback fails", async ({
  live,
  page,
}) => {
  const fixture = live.core.dependency_reads_project;
  await live.login(page);
  await loadProject(page, fixture.id);
  await page
    .getByRole("button", { name: `Open ${fixture.cancellation_blocker.key}`, exact: true })
    .click();
  await expect(dependencyPanel(page, "blocks")).toContainText("Condition: Pending");
  await draftForm(page).getByRole("button", { name: "Cancel task", exact: true }).click();
  await selectCancellation(page);
  let posts = 0;
  page.on("request", (request) => {
    if (request.url().endsWith(cancellationPath)) posts += 1;
  });
  const taskUrl = `${live.origin}/api/projects/${fixture.id}/tasks/${fixture.cancellation_blocker.id}`;
  await page.route(taskUrl, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await confirmCancellation(page).click();
      await expect(cancellationPanel(page)).toContainText("Cancellation accepted.");
      await expect(
        page.getByRole("button", { name: "Refresh cancelled data", exact: true }),
      ).toBeVisible();
      await expect(dependencyPanel(page, "blocks")).toContainText("Condition: Blocker cancelled");
    },
  );
  expect(posts).toBe(1);
  await page.unroute(taskUrl);
  await page.getByRole("button", { name: "Refresh cancelled data", exact: true }).click();
  await expect(cancellationPanel(page)).toContainText("Authoritative data refreshed.");
  expect(posts).toBe(1);
  await page.getByRole("button", { name: "Close cancellation", exact: true }).click();
  await dependencyPanel(page, "blocks")
    .getByRole("button", {
      name: `Open related ${fixture.cancellation_dependent.key}`,
      exact: true,
    })
    .click();
  await expect(dependencyPanel(page)).toContainText("Condition: Blocker cancelled");
  const client = await commandClient(live, { projectId: fixture.id });
  expect((await client.task(fixture.cancellation_dependent.id)).lifecycle).toBe("draft");
});

test("dependency read failures stay local, retain stale links, and explicit refresh recovers", async ({
  live,
  page,
}) => {
  await openDependencies(page, live);
  const pattern = `${dependencyUrl(live)}*`;
  await page.route(pattern, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable" } }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await dependencyPanel(page)
        .getByRole("button", { name: "Refresh dependencies", exact: true })
        .click();
      await expect(dependencyPanel(page)).toContainText(/stale/i);
    },
  );
  await expect(relatedButtons(page)).toHaveCount(20);
  await expect(relatedButtons(page, "blocks")).toHaveCount(20);
  await expect(draftForm(page)).toContainText(live.core.dependency_reads_project.root.title);
  await page.unroute(pattern);
  await dependencyPanel(page)
    .getByRole("button", { name: "Refresh dependencies", exact: true })
    .click();
  await expect(dependencyPanel(page)).not.toContainText(/stale/i);
  await expect(relatedButtons(page)).toHaveCount(20);
});

for (const destination of ["project", "logout"] as const) {
  test(`late dependency reads cannot repopulate the card after ${destination}`, async ({
    live,
    page,
  }) => {
    await openDependencies(page, live);
    // Capture the exact emitted URL, including the client's pagination parameters.
    let url = "";
    page.on("request", (request) => {
      if (request.url().startsWith(dependencyUrl(live))) url = request.url();
    });
    await dependencyPanel(page)
      .getByRole("button", { name: "Refresh dependencies", exact: true })
      .click();
    await expect.poll(() => url).not.toBe("");
    await expect(
      dependencyPanel(page).getByRole("button", { name: "Refresh dependencies", exact: true }),
    ).toBeEnabled();
    const held = await holdRealResponse(page, url);
    try {
      await dependencyPanel(page)
        .getByRole("button", { name: "Refresh dependencies", exact: true })
        .click();
      await held.ready;
      held.assertCaptured();
      if (destination === "project") await loadProject(page, live.core.empty_project.id);
      else {
        await page.getByRole("button", { name: "Log out", exact: true }).click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      }
      await held.releaseAfterCancellation();
      await expect(dependencyPanel(page)).toHaveCount(0);
      await expect(draftForm(page)).toHaveCount(0);
      if (destination === "project") {
        await loadProject(page, live.core.dependency_reads_project.id);
        await expect(
          page.getByRole("button", { name: "Back to previous task", exact: true }),
        ).toHaveCount(0);
      }
    } finally {
      await held.cleanup();
    }
  });
}

test("initial dependency errors can be retried without hiding task detail", async ({
  live,
  page,
}) => {
  const pattern = `${dependencyUrl(live)}*`;
  await page.route(pattern, (route) =>
    route.fulfill({ status: 503, json: { error: "Unavailable" } }),
  );
  await live.login(page);
  await loadProject(page, live.core.dependency_reads_project.id);
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 503 /,
    async () => {
      await page
        .getByRole("button", {
          name: `Open ${live.core.dependency_reads_project.root.key}`,
          exact: true,
        })
        .click();
      await expect(dependencyPanel(page).getByRole("alert")).toBeVisible();
    },
  );
  await expect(draftForm(page)).toContainText(live.core.dependency_reads_project.root.title);
  await expect(relatedButtons(page, "blocks")).toHaveCount(20);
  await page.unroute(pattern);
  await dependencyPanel(page)
    .getByRole("button", { name: "Refresh dependencies", exact: true })
    .click();
  await expect(relatedButtons(page)).toHaveCount(20);
});

test("synthetic invalid dependency cursor restarts only its direction on the real first page", async ({
  live,
  page,
}) => {
  await openDependencies(page, live);
  // Fault injection only: Core cursor semantics are covered by backend tests.
  const pattern = `${dependencyUrl(live)}*`;
  await page.route(pattern, (route) =>
    new URL(route.request().url()).searchParams.has("cursor")
      ? route.fulfill({ status: 409, json: { error: "Cursor expired", code: "cursor_invalid" } })
      : route.continue(),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 /,
    async () => {
      await dependencyPanel(page)
        .getByRole("button", { name: "Next dependencies", exact: true })
        .click();
      await expect(
        dependencyPanel(page).getByRole("button", {
          name: "Restart dependency pagination",
          exact: true,
        }),
      ).toBeVisible();
    },
  );
  await expect(relatedButtons(page, "blocks")).toHaveCount(20);
  await dependencyPanel(page)
    .getByRole("button", { name: "Restart dependency pagination", exact: true })
    .click();
  await expect(relatedButtons(page)).toHaveCount(20);
  await expect(
    dependencyPanel(page).getByRole("button", { name: "Previous dependencies", exact: true }),
  ).toBeDisabled();
});
