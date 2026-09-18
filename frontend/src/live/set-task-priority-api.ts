import { SetTaskPriorityRequestSchema } from "../contracts/set-task-priority.ts";
import type { TaskCommandAttempt } from "../contracts/task-command.ts";
import { sendTaskCommand } from "./task-command-api.ts";

export async function sendSetTaskPriority(
  fetcher: typeof fetch,
  attempt: TaskCommandAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = SetTaskPriorityRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  return sendTaskCommand(
    fetcher,
    "set_task_priority",
    attempt,
    request,
    token,
    signal,
    unauthorized,
  );
}
