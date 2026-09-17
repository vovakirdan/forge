// Synthetic transport failures prove browser recovery, not Core execution.
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { openPipelines, openVersion, pipelineCard, pipelineList } from "./pipeline-helpers";

test("synthetic Pipeline cursor expiration offers first-page recovery", async ({ live, page }) => {
  await openPipelines(page, live);
  await page.route(
    `**/api/projects/${live.core.pipelines_project.id}/pipelines?limit=20&cursor=*`,
    (route) =>
      route.fulfill({
        status: 409,
        json: { error: "Cursor expired", code: "cursor_invalid" },
      }),
  );
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/,
    async () => {
      await pipelineList(page).getByRole("button", { name: "Next page", exact: true }).click();
      await expect(
        pipelineList(page).getByRole("button", { name: "Restart pagination", exact: true }),
      ).toBeVisible();
    },
  );
  await pipelineList(page).getByRole("button", { name: "Restart pagination", exact: true }).click();
  await expect(
    pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
  ).toHaveCount(20);
});

test("synthetic oversized Pipeline list is not empty or truncated and failed refresh retains stale entries", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.pipelines_project.id);
  const url = `${live.origin}/api/projects/${live.core.pipelines_project.id}/pipelines?limit=20`;
  const unavailable = (route: import("@playwright/test").Route) =>
    route.fulfill({
      status: 502,
      json: { error: "Too large", code: "response_too_large" },
    });
  const error =
    /^Failed to load resource: the server responded with a status of 502 \(Bad Gateway\)$/;
  await page.route(url, unavailable);
  await live.allowConsoleErrors(error, async () => {
    await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
    await expect(pipelineList(page).getByRole("alert")).toContainText(/limit|too large/i);
    await expect(pipelineList(page)).not.toContainText("No pipeline versions in this project.");
    await expect(
      pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
    ).toHaveCount(0);
  });
  await page.unroute(url);
  await pipelineList(page)
    .getByRole("button", { name: "Refresh pipeline versions", exact: true })
    .click();
  await expect(
    pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
  ).toHaveCount(20);
  await page.route(url, unavailable);
  await live.allowConsoleErrors(error, async () => {
    await pipelineList(page)
      .getByRole("button", { name: "Refresh pipeline versions", exact: true })
      .click();
    await expect(pipelineList(page)).toContainText(/showing stale/i);
    await expect(
      pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
    ).toHaveCount(20);
  });
});

for (const failure of [
  { name: "missing", status: 404, console: "Not Found", code: undefined, message: /not found/i },
  {
    name: "oversized",
    status: 502,
    console: "Bad Gateway",
    code: "response_too_large",
    message: /limit|too large/i,
  },
  {
    name: "malformed",
    status: 200,
    console: "",
    code: undefined,
    message: /unsupported response/i,
  },
] as const) {
  test(`synthetic ${failure.name} Pipeline detail is explicit without partial success`, async ({
    live,
    page,
  }) => {
    await openPipelines(page, live);
    const url = `${live.origin}/api/projects/${live.core.pipelines_project.id}/pipelines/${live.core.pipelines_project.featured_id}`;
    await page.route(url, (route) =>
      route.fulfill({
        status: failure.status,
        json: { error: "Unavailable", code: failure.code },
      }),
    );
    const check = async () => {
      await openVersion(page, live.core.pipelines_project.featured_id).click();
      await expect(pipelineCard(page).getByRole("alert")).toContainText(failure.message);
      await expect(
        pipelineCard(page).getByRole("table", { name: "Stages", exact: true }),
      ).toHaveCount(0);
    };
    if (failure.status === 200) await check();
    else
      await live.allowConsoleErrors(
        new RegExp(
          `^Failed to load resource: the server responded with a status of ${failure.status} \\(${failure.console}\\)$`,
        ),
        check,
      );
    await page.unroute(url);
    await pipelineCard(page)
      .getByRole("button", { name: "Refresh pipeline version", exact: true })
      .click();
    await expect(pipelineCard(page)).toContainText(live.core.pipelines_project.featured_id);
    await expect(
      pipelineCard(page).getByRole("table", { name: "Stages", exact: true }),
    ).toBeVisible();
  });
}
