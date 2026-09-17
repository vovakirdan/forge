// Public DTO fixtures test rendering only; they do not claim stage/hook execution.
import {
  PipelineVersionViewSchema,
  type PipelineStageView,
  type PipelineVersionView,
} from "../../src/contracts/pipeline";
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { fact, openVersion, pipelineCard, pipelineList } from "./pipeline-helpers";

const id = (value: number) => `01900000-0000-7000-8000-${value.toString().padStart(12, "0")}`;
const hidden = "PRIVATE_HOOK_RUNTIME_CANARY";
const html = "<img src=x onerror=window.forgeInjected=true>";
const opaque = { private_runtime: hidden, url: "https://pipeline.invalid/never-fetch" };

function stage(key: string, executor: PipelineStageView["executor_kind"]): PipelineStageView {
  return {
    id: key,
    name: `${key} stage`,
    executor_kind: executor,
    outcomes: ["accepted", "rejected", "inconclusive"],
    instructions: html,
    workspace: null,
    acceptance_policy: null,
    system_action: null,
    ...opaque,
  };
}

function sample(): PipelineVersionView {
  const employee = stage("implement_change", "employee");
  employee.workspace = { kind: "git", access: "read_write", ...opaque };
  employee.acceptance_policy = {
    kind: "candidate_review",
    independent: true,
    verdicts: { accepted: "accepted", rejected: "rejected", inconclusive: "inconclusive" },
    ...opaque,
  };
  const human = stage("owner_conversation", "human");
  human.workspace = { kind: "filesystem", access: "read_only" };
  human.acceptance_policy = {
    kind: "candidate_review",
    independent: false,
    verdicts: { accepted: "accepted" },
  };
  const external = stage("outside_opinion", "external");
  external.workspace = { kind: "any", access: "read_only" };
  const integration = stage("integrate_change", "system");
  integration.system_action = {
    kind: "git_integration",
    outcomes: { applied: "merged", no_changes: "unchanged", stale_base: "retry", ...opaque },
    required_review_stages: [employee.id, "absent_review"],
    ...opaque,
  };
  const hook = stage("owner_hook", "system");
  hook.system_action = {
    kind: "project_hook",
    hook_version_id: id(80),
    outcomes: { passed: "green", failed: "red", timed_out: "late", skipped: "bypassed", ...opaque },
    ...opaque,
  };
  return PipelineVersionViewSchema.parse({
    id: id(1),
    pipeline_id: id(2),
    version: 1,
    name: `Synthetic catalog ${html}`,
    catalog_revision: 2,
    default_version_id: id(1),
    latest_version: 2,
    deleted_at: null,
    task_kinds: ["analysis", "delivery"],
    entry_stage_id: employee.id,
    max_stage_visits: null,
    stages: [employee, human, external, integration, hook],
    transitions: [
      {
        from_stage_id: employee.id,
        outcome: "accepted",
        target: { kind: "stage", stage_id: human.id, ...opaque },
        artifact_requirements: [
          { kind: "migration_plan", minimum_count: 2, scope: "current_stage", ...opaque },
        ],
        ...opaque,
      },
      {
        from_stage_id: human.id,
        outcome: "accepted",
        target: { kind: "done" },
        artifact_requirements: [{ kind: "report", minimum_count: 1, scope: "task_history" }],
      },
      { from_stage_id: external.id, outcome: "rejected", target: { kind: "cancelled" } },
    ],
    ...opaque,
  });
}

test("synthetic Pipeline stages render all executor and policy variants as safe read-only fields", async ({
  live,
  page,
}) => {
  const value = sample();
  const requests: { url: string; method: string }[] = [];
  page.on("request", (request) => requests.push({ url: request.url(), method: request.method() }));
  const prefix = `${live.origin}/api/projects/${live.core.pipelines_project.id}/pipelines`;
  await page.route(`${prefix}?limit=20`, (route) => route.fulfill({ json: { items: [value] } }));
  await page.route(`${prefix}/${value.id}`, (route) => route.fulfill({ json: value }));
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.pipelines_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  await openVersion(page, value.id).click();
  const card = pipelineCard(page);
  await expect(card).toContainText(value.id);
  await fact(card, "Default version", "Yes");
  await fact(card, "Latest version", "No");
  await fact(card, "Deletion state", "Not deleted");
  await expect(card).toContainText("Not configured; this does not promise unlimited execution.");
  const selected = card.getByRole("region", { name: "Selected stage", exact: true });
  for (const item of value.stages) {
    await card.getByRole("button", { name: `Inspect stage ${item.id}`, exact: true }).click();
    await fact(selected, "Stage ID", item.id);
    await fact(selected, "Executor", item.executor_kind);
    await expect(
      selected.getByRole("region", { name: "Stage instructions", exact: true }),
    ).toContainText(html);
    const workspace = selected.getByRole("region", { name: "Workspace policy", exact: true });
    if (item.workspace) {
      await fact(workspace, "Workspace kind", item.workspace.kind);
      await fact(workspace, "Workspace access", item.workspace.access);
    } else await expect(workspace).toContainText("a WorkSurface may still be provided");
    const acceptance = selected.getByRole("region", { name: "Acceptance policy", exact: true });
    if (item.acceptance_policy) {
      await fact(acceptance, "Acceptance kind", "candidate_review");
      await fact(
        acceptance,
        "Independent review",
        item.acceptance_policy.independent ? "Yes" : "No",
      );
      for (const [outcome, verdict] of Object.entries(item.acceptance_policy.verdicts))
        await fact(acceptance, outcome, verdict);
    } else await expect(acceptance).toContainText("this does not imply automatic acceptance");
    const action = selected.getByRole("region", { name: "System action", exact: true });
    if (item.system_action?.kind === "git_integration") {
      await fact(action, "Action kind", "git_integration");
      await fact(action, "Applied outcome", "merged");
      await fact(action, "No changes outcome", "unchanged");
      await fact(action, "Stale base outcome", "retry");
      await fact(
        action,
        "Required review stages",
        /implement_change stage \(implement_change\).*absent_review — stage unavailable/s,
      );
    } else if (item.system_action?.kind === "project_hook") {
      await fact(action, "Action kind", "project_hook");
      await fact(action, "Hook version ID", id(80));
      for (const [label, outcome] of [
        ["Passed", "green"],
        ["Failed", "red"],
        ["Timed out", "late"],
        ["Skipped", "bypassed"],
      ] as const)
        await fact(action, `${label} outcome`, outcome);
    } else await expect(action).toContainText("Not configured on this stage.");
    await expect(card).not.toContainText(hidden);
    await expect(card.getByRole("link")).toHaveCount(0);
  }
  expect(await page.evaluate(() => "forgeInjected" in window)).toBe(false);
  expect(requests.every((request) => new URL(request.url).origin === live.origin)).toBe(true);
  expect(requests.some((request) => request.url.includes("never-fetch"))).toBe(false);
  expect(
    requests
      .filter((request) => request.url.includes("/pipelines"))
      .every((request) => request.method === "GET"),
  ).toBe(true);
});

test("synthetic unknown stage references keep their IDs and refreshed catalog metadata is not frozen", async ({
  live,
  page,
}) => {
  const value = sample();
  value.entry_stage_id = "missing_entry";
  value.transitions = [
    {
      from_stage_id: "missing_source",
      outcome: "continue",
      target: { kind: "stage", stage_id: "missing_target" },
    },
  ];
  const prefix = `${live.origin}/api/projects/${live.core.pipelines_project.id}/pipelines`;
  let refreshed = false;
  await page.route(`${prefix}?limit=20`, (route) => route.fulfill({ json: { items: [value] } }));
  await page.route(`${prefix}/${value.id}`, (route) =>
    route.fulfill({
      json: refreshed
        ? {
            ...value,
            name: "Renamed catalog",
            catalog_revision: 3,
            default_version_id: id(3),
            latest_version: 3,
            deleted_at: "2026-09-17T08:00:00Z",
          }
        : value,
    }),
  );
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.pipelines_project.id);
  await page.getByRole("button", { name: "Pipeline versions", exact: true }).click();
  await openVersion(page, value.id).click();
  const card = pipelineCard(page);
  for (const reference of ["missing_entry", "missing_source", "missing_target"])
    await expect(card).toContainText(`${reference} — stage unavailable`);
  const selected = card.getByRole("region", { name: "Selected stage", exact: true });
  await expect(selected).toContainText("missing_entry");
  await expect(
    selected.getByRole("region", { name: "Stage instructions", exact: true }),
  ).toHaveCount(0);
  refreshed = true;
  await card.getByRole("button", { name: "Refresh pipeline version", exact: true }).click();
  await fact(card, "Name", "Renamed catalog");
  await fact(card, "Default version", "No");
  await fact(card, "Deletion state", "Soft deleted");
  await fact(card, "Version", "1");
  await expect(pipelineList(page)).not.toContainText("Renamed catalog");
});
