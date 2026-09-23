import { test, expect } from "./fixtures";
import { holdRealResponse, loadProject } from "./task-helpers";
import { openPipelines, openVersion, pipelineCard, pipelineList } from "./pipeline-helpers";

for (const destination of ["version", "page", "section", "logout"] as const) {
  test(`switching Pipeline ${destination} cancels a delayed real detail and cannot restore it`, async ({
    live,
    page,
  }) => {
    await openPipelines(page, live);
    const project = live.core.pipelines_project;
    const held = await holdRealResponse(
      page,
      `${live.origin}/api/projects/${project.id}/pipelines/${project.featured_id}`,
    );
    try {
      await openVersion(page, project.featured_id).click();
      await held.ready;
      held.assertCaptured();
      if (destination === "version") {
        await openVersion(page, project.second_id).click();
        await expect(pipelineCard(page)).toContainText("Replacement migration strategy v2");
      } else if (destination === "page") {
        await pipelineList(page).getByRole("button", { name: "Next page", exact: true }).click();
        await expect(
          pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
        ).not.toHaveCount(0);
        await expect(openVersion(page, project.featured_id)).toHaveCount(0);
      } else if (destination === "section") {
        await page.getByRole("button", { name: "Tasks", exact: true }).click();
        await expect(page.getByRole("region", { name: "Tasks", exact: true })).toBeVisible();
      } else {
        await page.getByRole("button", { name: "Log out", exact: true }).click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      }
      await held.releaseAfterCancellation();
      if (destination === "version") {
        await expect(pipelineCard(page)).toContainText("Replacement migration strategy v2");
        await expect(
          pipelineCard(page).getByRole("region", { name: "Selected stage", exact: true }),
        ).not.toContainText(project.stage_name);
      } else {
        await expect(pipelineCard(page)).toHaveCount(0);
      }
      if (destination === "section") {
        await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
        await expect(
          pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
        ).toHaveCount(20);
        await expect(pipelineCard(page)).toHaveCount(0);
      }
    } finally {
      await held.cleanup();
    }
  });
}

test("switching Project cancels a delayed Pipeline list without recreating the old query", async ({
  live,
  page,
}) => {
  await openPipelines(page, live);
  const url = `${live.origin}/api/projects/${live.core.pipelines_project.id}/pipelines?limit=20`;
  let oldRequests = 0;
  page.on("request", (request) => {
    if (request.url() === url) oldRequests += 1;
  });
  const held = await holdRealResponse(page, url);
  try {
    await pipelineList(page)
      .getByRole("button", { name: "Refresh pipeline versions", exact: true })
      .click();
    await held.ready;
    held.assertCaptured();
    await loadProject(page, live.core.other_pipelines_project.id);
    await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
    await expect(pipelineList(page)).toContainText(live.core.other_pipelines_project.version_id);
    await held.releaseAfterCancellation();
    await expect(
      pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
    ).toHaveCount(1);
    await expect(pipelineList(page)).not.toContainText(live.core.pipelines_project.featured_id);
    expect(oldRequests, "Scope change does not recreate the previous observed query").toBe(1);
  } finally {
    await held.cleanup();
  }
});

test("real 401 revocation cancels a pending Pipeline read and clears the session", async ({
  live,
  page,
}) => {
  await openPipelines(page, live);
  const project = live.core.pipelines_project;
  const held = await holdRealResponse(
    page,
    `${live.origin}/api/projects/${project.id}/pipelines/${project.featured_id}`,
  );
  try {
    await openVersion(page, project.featured_id).click();
    await held.ready;
    held.assertCaptured();
    const popup = page.waitForEvent("popup");
    await page.evaluate(() => {
      window.open(window.location.href, "_blank");
    });
    const copied = await popup;
    await expect(copied.getByText("Connected to Forge", { exact: true })).toBeVisible();
    const revoked = copied.waitForResponse(
      (response) => response.url() === `${live.origin}/api/auth/logout`,
    );
    await copied.getByRole("button", { name: "Log out", exact: true }).click();
    expect((await revoked).status()).toBe(204);
    await live.allowConsoleErrors(
      /^Failed to load resource: the server responded with a status of 401 \(Unauthorized\)$/,
      async () => {
        await pipelineList(page)
          .getByRole("button", { name: "Refresh pipeline versions", exact: true })
          .click();
        await expect(
          page.getByRole("heading", { name: "Connect to Forge", exact: true }),
        ).toBeVisible();
      },
    );
    await held.releaseAfterCancellation();
    await expect(pipelineList(page)).toHaveCount(0);
    await expect(pipelineCard(page)).toHaveCount(0);
  } finally {
    await held.cleanup();
  }
});
