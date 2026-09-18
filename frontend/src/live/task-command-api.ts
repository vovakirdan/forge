import { z } from "zod";
import { TaskCommandReceiptSchema, IdempotencyKeySchema } from "../contracts/task-command.ts";
import type { TaskCommandAttempt } from "../contracts/task-command.ts";
import type { CreateTaskAttempt } from "../contracts/create-task.ts";
import { LiveCommandError } from "./command-error.ts";

const errorSchema = z.object({ error: z.string(), code: z.string() }).strict();

async function boundedJson(response: Response): Promise<unknown> {
  const reader = response.body?.getReader();
  if (!reader) throw new LiveCommandError("outcome_unknown");
  let bytes = 0;
  let text = "";
  const decoder = new TextDecoder("utf-8", { fatal: true });
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > 64 * 1024) throw new LiveCommandError("outcome_unknown");
      text += decoder.decode(value, { stream: true });
    }
    return JSON.parse(text + decoder.decode()) as unknown;
  } finally {
    void reader.cancel().catch(() => undefined);
  }
}

export async function sendTaskCommand(
  fetcher: typeof fetch,
  command: "amend_draft" | "set_task_priority" | "create_task" | "approve_task" | "cancel_task",
  attempt: TaskCommandAttempt | CreateTaskAttempt,
  request: { expected_revision: number; payload: object },
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  // Validate before transport; never reconstruct the immutable retry body.
  IdempotencyKeySchema.parse(attempt.key);
  if (
    (command !== "create_task" &&
      (!("taskId" in attempt) ||
        !("task_id" in request.payload) ||
        request.payload.task_id !== attempt.taskId)) ||
    new TextEncoder().encode(attempt.body).byteLength > 512 * 1024
  )
    throw new LiveCommandError("invalid_request");
  let response: Response;
  try {
    response = await fetcher(`/api/commands/${command}`, {
      method: "POST",
      signal,
      credentials: "omit",
      redirect: "error",
      cache: "no-store",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
        "Idempotency-Key": attempt.key,
      },
      body: attempt.body,
    });
  } catch {
    throw new LiveCommandError("outcome_unknown");
  }
  if (response.status === 401) throw unauthorized();
  let value: unknown;
  try {
    value = await boundedJson(response);
  } catch {
    throw new LiveCommandError("outcome_unknown");
  }
  if (!response.ok) {
    const parsed = errorSchema.safeParse(value);
    const code = parsed.success ? parsed.data.code : null;
    if (response.status === 400 && code === "invalid_request") throw new LiveCommandError(code);
    if (response.status === 409 && (code === "stale_revision" || code === "idempotency_conflict"))
      throw new LiveCommandError(code);
    if (response.status === 422 && code === "validation_failed") throw new LiveCommandError(code);
    if (response.status === 404 && code === "not_found") throw new LiveCommandError(code);
    if (response.status === 403 && code === "forbidden") throw new LiveCommandError(code);
    throw new LiveCommandError("outcome_unknown");
  }
  const receipt = TaskCommandReceiptSchema.safeParse(value);
  if (
    response.status !== 200 ||
    !receipt.success ||
    (command !== "create_task" &&
      (!("taskId" in attempt) ||
        receipt.data.resource.id.toLowerCase() !== attempt.taskId.toLowerCase())) ||
    (command === "cancel_task"
      ? receipt.data.project_revision <= request.expected_revision
      : receipt.data.project_revision !== request.expected_revision + 1)
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
