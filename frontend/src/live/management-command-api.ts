import {
  ManagementReceiptSchema,
  managementAttempt,
  type ManagementAttempt,
} from "../contracts/management-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";
const kind = {
  start_project_execution: "project",
  stop_project_execution: "project",
  pause_task: "task",
  resume_task: "task",
  stop_employee: "employee",
  submit_human_resolution: "escalation",
  reroute_escalation: "escalation",
  accept_run_recovery_assessment: "run",
} as const;
export async function sendManagementCommand(
  fetcher: typeof fetch,
  attempt: ManagementAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const parsed = JSON.parse(attempt.body) as {
      project_id: string;
      expected_revision: number;
      payload: unknown;
    };
    const checked = managementAttempt(attempt.action, parsed, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.resourceId !== attempt.resourceId ||
      checked.projectId !== attempt.projectId
    )
      throw Error("changed");
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const { status, value } = await postCommand(
    fetcher,
    attempt.action,
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = ManagementReceiptSchema.safeParse(value);
  const revisionOkay =
    receipt.success &&
    (attempt.action === "pause_task" ||
    attempt.action === "stop_employee" ||
    attempt.action === "stop_project_execution" ||
    attempt.action === "submit_human_resolution" ||
    attempt.action === "reroute_escalation" ||
    attempt.action === "accept_run_recovery_assessment"
      ? receipt.data.project_revision > attempt.expectedRevision
      : receipt.data.project_revision === attempt.expectedRevision + 1);
  if (
    status !== 200 ||
    !receipt.success ||
    !revisionOkay ||
    receipt.data.resource.kind !== kind[attempt.action] ||
    receipt.data.resource.id.toLowerCase() !== attempt.resourceId.toLowerCase()
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
