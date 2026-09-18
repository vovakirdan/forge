import { CancelTaskRequestSchema } from "../contracts/cancel-task.ts";
import type { TaskCommandAttempt } from "../contracts/task-command.ts";
import { sendTaskCommand } from "./task-command-api.ts";

export async function sendCancelTask(
  fetcher: typeof fetch,
  attempt: TaskCommandAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = CancelTaskRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  return sendTaskCommand(fetcher, "cancel_task", attempt, request, token, signal, unauthorized);
}
