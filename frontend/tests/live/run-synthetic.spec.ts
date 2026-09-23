// These browser fixtures cover the public DTO union, not execution of these purposes.
import type { Locator } from "@playwright/test";
import type { ExecutionAssignment } from "../../src/contracts/execution-assignment";
import { RunDetailViewSchema, type RunDetailView } from "../../src/contracts/run";
import { presentRun } from "../../src/presentation/run";
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { runCard, runList } from "./run-helpers";
import { populatedRunDiagnosticsFixture } from "../../src/contracts/run-fixtures";

const id = (value: number) => `01900000-0000-7000-8000-${value.toString().padStart(12, "0")}`;
const streamCanary = "stdout <img src=x onerror=window.forgeInjected=true>";
const assignments: ExecutionAssignment[] = [
  { purpose: "task_stage", owner: { task_id: id(101), queue_entry_id: id(102), stage_id: "work" } },
  {
    purpose: "communication",
    owner: { assignment_id: id(103), thread_id: id(104), source_message_id: id(105) },
  },
  {
    purpose: "resolution",
    owner: { assignment_id: id(106), escalation_id: id(107), lease_generation: 3 },
  },
  {
    purpose: "hook",
    owner: {
      invocation_id: id(108),
      task_id: id(109),
      pipeline_version_id: id(110),
      stage_id: "acceptance",
      stage_visit: 2,
      candidate_proposal_id: id(111),
    },
  },
  {
    purpose: "system_job",
    owner: { job_id: id(112), attempt_id: id(113), generation: 4, kind: "summarization" },
  },
  {
    purpose: "system_job",
    owner: { job_id: id(114), attempt_id: id(115), generation: 5, kind: "onboarding" },
  },
];

function detail(assignment: ExecutionAssignment, index: number): RunDetailView {
  return RunDetailViewSchema.parse({
    id: id(index + 1),
    assignment,
    task_id: assignment.purpose === "task_stage" ? assignment.owner.task_id : null,
    employee_id:
      assignment.purpose === "hook" || assignment.purpose === "system_job" ? null : id(200),
    stage_id: assignment.purpose === "task_stage" ? assignment.owner.stage_id : null,
    attempt: 1,
    desired_state: index === 1 ? "force_stop_requested" : "stop_requested",
    observed_state: index === 1 ? "unknown" : "running",
    lease_fencing_token: 7,
    environment_epoch: 8,
    last_observed_sequence: 0,
    run_spec_version: 2,
    diagnostics:
      index === 0
        ? {
            ...populatedRunDiagnosticsFixture,
            evidence: [
              populatedRunDiagnosticsFixture.evidence[0],
              { ...populatedRunDiagnosticsFixture.evidence[0], id: id(300) },
            ],
            streams: [
              { stream: streamCanary, incomplete: true },
              { stream: "stderr", incomplete: false },
            ],
          }
        : {
            runtime_report: index === 1 ? { available: true } : null,
            handoff: null,
            proxy_usage: null,
            git_source: null,
            incidents: [],
            evidence: [],
            streams: [],
          },
  });
}

async function fact(region: Locator, label: string, value: string) {
  // Semantic definition-list adjacency; no layout classes or positional index.
  await expect(region.getByText(label, { exact: true }).locator("+ dd")).toHaveText(value);
}

test("synthetic Run purposes keep nullable context, independent states and opaque diagnostics distinct", async ({
  live,
  page,
}) => {
  const samples = assignments.map(detail);
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  const prefix = `${live.origin}/api/projects/${live.core.runs_project.id}/runs`;
  await page.route(`${prefix}?limit=20`, (route) =>
    route.fulfill({
      json: { items: samples.map(({ diagnostics: _diagnostics, ...run }) => run) },
    }),
  );
  for (const run of samples)
    await page.route(`${prefix}/${run.id}`, (route) => route.fulfill({ json: run }));
  for (const run of samples) {
    await page.route(`${prefix}/${run.id}/context`, (route) =>
      route.fulfill({ json: { availability: "unavailable", run_id: run.id, coordinates: null } }),
    );
    await page.route(`${prefix}/${run.id}/evidence?limit=20`, (route) =>
      route.fulfill({ json: { items: [] } }),
    );
  }
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await loadProject(page, live.core.runs_project.id);
  await page.getByRole("button", { name: "Runs", exact: true }).click();
  await expect(runList(page).getByRole("button", { name: /^Open Run / })).toHaveCount(
    samples.length,
  );
  for (const [index, run] of samples.entries()) {
    await runList(page)
      .getByRole("button", { name: `Open Run ${run.id}`, exact: true })
      .click();
    const card = runCard(page);
    await expect(card).toContainText(run.id);
    const labels = presentRun(run).presentation;
    await fact(card, "Purpose", `${labels.purposeLabel} (${run.assignment.purpose})`);
    await fact(card, "Desired state", `${labels.desiredStateLabel} (${run.desired_state})`);
    await fact(card, "Observed state", `${labels.observedStateLabel} (${run.observed_state})`);
    await fact(card, "Task ID", run.task_id ?? "Not assigned");
    await fact(card, "Employee ID", run.employee_id ?? "Not assigned");
    await fact(card, "Stage ID", run.stage_id ?? "Not assigned");
    const owner = card.getByRole("region", { name: "Assignment owner", exact: true });
    for (const value of Object.values(run.assignment.owner))
      await expect(owner).toContainText(String(value));
    if (run.assignment.purpose === "hook") {
      await fact(owner, "Context Task ID", run.assignment.owner.task_id);
      await fact(owner, "Context stage ID", run.assignment.owner.stage_id);
      await expect(owner.getByText("Owner Task ID", { exact: true })).toHaveCount(0);
      await expect(owner.getByText("Owner stage ID", { exact: true })).toHaveCount(0);
    }
    if (run.assignment.purpose === "system_job")
      await fact(
        owner,
        "System job kind",
        `${labels.systemJobKindLabel} (${run.assignment.owner.kind})`,
      );
    const diagnostics = card.getByRole("region", { name: "Diagnostics", exact: true });
    await fact(diagnostics, "Runtime report", index <= 1 ? "Available" : "Unavailable");
    for (const label of ["Handoff", "Proxy usage", "Git source"])
      await fact(diagnostics, label, index === 0 ? "Available" : "Unavailable");
    await fact(diagnostics, "Loaded incidents", index === 0 ? "1" : "0");
    await fact(diagnostics, "Loaded evidence", index === 0 ? "2" : "0");
    await fact(diagnostics, "Loaded incomplete streams", index === 0 ? "1" : "0");
    if (index === 0) {
      await expect(diagnostics).toContainText(streamCanary);
      await expect(diagnostics.getByText("Yes", { exact: true })).toHaveCount(1);
      await expect(diagnostics.getByText("No", { exact: true })).toHaveCount(1);
    }
    await expect(card.getByRole("link")).toHaveCount(0);
  }
  expect(await page.evaluate(() => "forgeInjected" in window)).toBe(false);
  expect(requests.every((url) => new URL(url).origin === live.origin)).toBe(true);
  expect(requests.some((url) => url.includes("diagnostic.invalid"))).toBe(false);
});
