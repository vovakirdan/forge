import {
  managerPlanningAttempt,
  ManagerPlanningReceiptSchema,
  type ManagerPlanningAttempt,
} from "../contracts/manager-planning.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendManagerPlanning(
  fetcher: typeof fetch,
  attempt: ManagerPlanningAttempt,
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
    const checked = managerPlanningAttempt(attempt.action, parsed, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.resourceId !== attempt.resourceId ||
      checked.expectedRevision !== attempt.expectedRevision
    )
      throw Error("attempt changed");
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
  const receipt = ManagerPlanningReceiptSchema.safeParse(value);
  const kind =
    attempt.action === "schedule_task_resume" || attempt.action === "cancel_task_resume"
      ? "task_resume_schedule"
      : "task";
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedRevision + 1 ||
    receipt.data.resource.kind !== kind ||
    (attempt.resourceId &&
      receipt.data.resource.id.toLowerCase() !== attempt.resourceId.toLowerCase())
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
