import { ApproveTaskRequestSchema } from "../contracts/approve-task.ts";
import type { TaskCommandAttempt } from "../contracts/task-command.ts";
import { sendTaskCommand } from "./task-command-api.ts";

export async function sendApproveTask(
  fetcher: typeof fetch,
  attempt: TaskCommandAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = ApproveTaskRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  return sendTaskCommand(fetcher, "approve_task", attempt, request, token, signal, unauthorized);
}
