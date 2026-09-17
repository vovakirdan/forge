import {
  PipelineVersionListResponseSchema,
  PipelineVersionViewSchema,
} from "../../src/contracts/pipeline";
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { fact, openPipelines, openVersion, pipelineCard, pipelineList } from "./pipeline-helpers";

test("real Pipeline pages exceed 64 KiB, preserve server order and fetch selected definitions freshly", async ({
  live,
  page,
}) => {
  const project = live.core.pipelines_project;
  const token = await live.bearer();
  const get = async (tail: string) => {
    const response = await fetch(`${live.origin}/api/projects/${project.id}/pipelines${tail}`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(6000),
    });
    expect(response.status).toBe(200);
    const body = await response.text();
    expect(
      [...live.secrets].some((secret) => body.includes(secret)),
      "No issued secret in Pipeline body",
    ).toBe(false);
    return { bytes: Buffer.byteLength(body), value: JSON.parse(body) as unknown };
  };
  const response = await get("?limit=20");
  expect(response.bytes).toBeGreaterThan(64 * 1024);
  expect(response.bytes).toBeLessThanOrEqual(1024 * 1024);
  const first = PipelineVersionListResponseSchema.parse(response.value);
  expect(first.items).toHaveLength(20);
  expect(first.next_cursor).toBeTruthy();
  const old = PipelineVersionViewSchema.parse((await get(`/${project.featured_id}`)).value);
  const newer = PipelineVersionViewSchema.parse((await get(`/${project.second_id}`)).value);
  const deleted = PipelineVersionViewSchema.parse((await get(`/${project.deleted_id}`)).value);
  expect(old.version).toBe(1);
  expect(old.latest_version).toBe(2);
  expect(old.default_version_id).toBe(old.id);
  expect(newer.pipeline_id).toBe(old.pipeline_id);
  expect(newer.default_version_id).toBe(old.id);
  expect(deleted.deleted_at).not.toBeNull();

  await openPipelines(page, live);
  const list = pipelineList(page);
  await expect(list.getByRole("button", { name: /^Open Pipeline version / })).toHaveText(
    first.items.map((version) => `${version.name} · version ${version.version}`),
  );
  let detailReads = 0;
  page.on("request", (request) => {
    if (request.url() === `${live.origin}/api/projects/${project.id}/pipelines/${old.id}`)
      detailReads += 1;
  });
  await openVersion(page, old.id).click();
  const card = pipelineCard(page);
  await expect(card).toContainText(old.id);
  expect(detailReads, "Opening a card reads Core despite the full definition in the list").toBe(1);
  await fact(card, "Version", "1");
  await fact(card, "Pipeline ID", project.pipeline_id);
  await expect(card.getByRole("table", { name: "Stages", exact: true })).toContainText(
    project.stage_name,
  );
  const transitions = card.getByRole("table", { name: "Transitions", exact: true });
  for (const text of [
    "planned",
    "withdrawn",
    "accepted",
    "peer_exchange",
    "done",
    "cancelled",
    "migration_plan",
    "review_evidence",
    "current_stage",
    "task_history",
  ])
    await expect(transitions).toContainText(text);
  await expect(card.getByRole("region", { name: "Selected stage", exact: true })).toContainText(
    project.stage_name,
  );
  await expect(
    card.getByRole("button", { name: `Inspect stage ${project.stage_id}`, exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(card).toContainText("<img src=x onerror=window.forgeInjected=true>");
  expect(await page.evaluate(() => "forgeInjected" in window)).toBe(false);
  await openVersion(page, deleted.id).click();
  await expect(card).toContainText("Archived migration catalog");
  await fact(card, "Deletion state", /deleted/i);
  await list.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open Pipeline version / })).toHaveCount(
    project.total - 20,
  );
  await expect(card).toHaveCount(0);
  await expect(list.getByRole("button", { name: "Next page", exact: true })).toBeDisabled();
  await list.getByRole("button", { name: "Previous page", exact: true }).click();
  await expect(list.getByRole("button", { name: /^Open Pipeline version / })).toHaveText(
    first.items.map((version) => `${version.name} · version ${version.version}`),
  );
  await page.reload();
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await expect(list).toHaveCount(0);
});

test("real Pipeline scopes reject foreign versions and section changes reset detail and pagination", async ({
  live,
  page,
}) => {
  await openPipelines(page, live);
  await openVersion(page, live.core.pipelines_project.featured_id).click();
  await expect(pipelineCard(page)).toContainText(live.core.pipelines_project.featured_id);
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(pipelineCard(page)).toHaveCount(0);
  await expect(pipelineList(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  await expect(
    pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
  ).toHaveCount(20);
  await expect(pipelineCard(page)).toHaveCount(0);
  await loadProject(page, live.core.other_pipelines_project.id);
  await expect(page.getByRole("button", { name: "Tasks", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  await expect(
    pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
  ).toHaveCount(1);
  await expect(pipelineList(page)).toContainText(live.core.other_pipelines_project.version_id);
  await expect(pipelineList(page)).not.toContainText(live.core.pipelines_project.featured_id);
  const token = await live.bearer();
  for (const [path, status] of [
    [
      `${live.core.other_pipelines_project.id}/pipelines/${live.core.pipelines_project.featured_id}`,
      404,
    ],
    [`${live.core.pipelines_project.id}/pipelines?limit=20&cursor=missing`, 409],
  ] as const) {
    const response = await fetch(`${live.origin}/api/projects/${path}`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(6000),
    });
    expect(response.status).toBe(status);
    if (status === 409) expect(await response.json()).toMatchObject({ code: "cursor_invalid" });
  }
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  await expect(pipelineList(page)).toContainText("No pipeline versions in this project.");
  await expect(
    pipelineList(page).getByRole("button", { name: /^Open Pipeline version / }),
  ).toHaveCount(0);
});

test("Pipeline offline refresh preserves stale detail, keyboard recovery works at 375px", async ({
  live,
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openPipelines(page, live);
  const open = openVersion(page, live.core.pipelines_project.featured_id);
  await open.focus();
  await page.keyboard.press("Enter");
  const card = pipelineCard(page);
  await expect(card).toContainText(live.core.pipelines_project.featured_id);
  await page.context().setOffline(true);
  await live.allowConsoleErrors(
    /^Failed to load resource: net::ERR_INTERNET_DISCONNECTED$/,
    async () => {
      const failed = page.waitForEvent("console", {
        predicate: (message) =>
          message.text() === "Failed to load resource: net::ERR_INTERNET_DISCONNECTED",
      });
      await card.getByRole("button", { name: "Refresh pipeline version", exact: true }).click();
      await expect(card).toContainText(/showing stale/i);
      await expect(card).toContainText(live.core.pipelines_project.featured_id);
      await failed;
    },
  );
  await page.context().setOffline(false);
  await card.getByRole("button", { name: "Refresh pipeline version", exact: true }).click();
  await expect(
    card.getByRole("button", { name: "Refresh pipeline version", exact: true }),
  ).toBeEnabled();
  await expect(card).not.toContainText(/showing stale/i);
  await card.getByRole("button", { name: "Inspect stage peer_exchange", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(card.getByRole("region", { name: "Selected stage", exact: true })).toContainText(
    "Read the plan and provide an external verdict.",
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await card.getByRole("button", { name: "Close pipeline version", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(card).toHaveCount(0);
});
