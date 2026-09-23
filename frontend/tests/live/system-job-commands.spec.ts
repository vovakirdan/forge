import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";
import { randomUUID } from "node:crypto";
import { ProjectViewSchema } from "../../src/contracts/project";
import { EmployeeCommandReceiptSchema } from "../../src/contracts/create-employee";

test("System Jobs shows an empty bounded history without inventing attempts", async ({
  live,
  page,
}) => {
  await live.login(page);
  await loadProject(page, live.core.empty_project.id);
  await page.getByRole("button", { name: "System Jobs", exact: true }).click();
  const jobs = page.getByRole("region", { name: "System Jobs", exact: true });
  await expect(jobs.getByRole("region", { name: "System Job records" })).toContainText(
    "No System Jobs in this response.",
  );
  await expect(jobs).toContainText("not a complete history or an attempt list");
});

test("owner explicitly skips pending Employee onboarding with an audited reason", async ({
  live,
  page,
}) => {
  const projectId = live.core.employee_creation_project.id;
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const project = ProjectViewSchema.parse(
    await (await fetch(`${live.origin}/api/projects/${projectId}`, { headers })).json(),
  );
  const name = `Onboarding ${randomUUID()}`;
  const created = await fetch(`${live.origin}/api/commands/create_employee`, {
    method: "POST",
    headers: {
      ...headers,
      Origin: live.origin,
      "Content-Type": "application/json",
      "Idempotency-Key": randomUUID(),
    },
    body: JSON.stringify({
      project_id: projectId,
      expected_revision: project.revision,
      payload: { name, role: "reviewer", stage_eligibility: { mode: "any" } },
    }),
  });
  expect(created.status).toBe(200);
  EmployeeCommandReceiptSchema.parse(await created.json());
  await live.login(page);
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await page.getByRole("button", { name: `Open Employee ${name}` }).click();
  const profile = page.getByRole("region", { name: "Employee profile" });
  const onboarding = profile.getByRole("region", { name: "Employee onboarding" });
  await expect(onboarding).toContainText("pending");
  await onboarding
    .getByLabel("Reason for skipping onboarding")
    .fill("Owner confirmed local onboarding waiver");
  await onboarding.getByRole("button", { name: "Skip Employee onboarding" }).click();
  const confirmation = onboarding.getByRole("alertdialog");
  await expect(confirmation.getByRole("heading")).toBeFocused();
  await confirmation.getByRole("button", { name: "Confirm" }).click();
  await expect(onboarding).toContainText("skipped");
  await expect(onboarding).toContainText("explicitly skipped");
});
