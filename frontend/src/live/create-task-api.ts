import { CreateTaskRequestSchema, type CreateTaskAttempt } from "../contracts/create-task.ts";
import { sendTaskCommand } from "./task-command-api.ts";

export async function sendCreateTask(
  fetcher: typeof fetch,
  attempt: CreateTaskAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = CreateTaskRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  return sendTaskCommand(fetcher, "create_task", attempt, request, token, signal, unauthorized);
}
