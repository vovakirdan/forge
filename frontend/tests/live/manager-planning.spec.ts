import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import { loadProject } from "./task-helpers";

test("owner pins the next Run to an eligible Employee without moving active work", async ({
  live,
  page,
}) => {
  const projectId = live.core.project_id;
  const token = await live.bearer();
  const headers = { Authorization: `Bearer ${token}` };
  const project = (await (
    await fetch(`${live.origin}/api/projects/${projectId}`, { headers })
  ).json()) as { revision: number };
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
      payload: {
        name: `Planner ${randomUUID()}`,
        role: "developer",
        stage_eligibility: { mode: "any" },
      },
    }),
  });
  expect(created.status).toBe(200);
  const receipt = (await created.json()) as { resource: { id: string } };
  await live.login(page);
  await loadProject(page, projectId);
  await page.getByRole("button", { name: "Management", exact: true }).click();
  const panel = page.getByRole("region", { name: "Next Run and resume planning" });
  await panel.getByLabel("Task ID").fill(live.core.tasks.second_id);
  await panel.getByLabel("Employee ID").fill(receipt.resource.id);
  await panel.getByRole("button", { name: "Review action" }).click();
  await expect(
    panel.getByRole("alertdialog", { name: "Confirm management planning" }),
  ).toBeVisible();
  await panel.getByRole("alertdialog").getByRole("button", { name: "Confirm" }).click();
  await expect(panel.getByLabel("Task ID")).toHaveValue("");
  await expect(page.getByRole("region", { name: "Next Run constraints" })).toContainText(
    receipt.resource.id,
  );
});
