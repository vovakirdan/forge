import {
  CreateDependencyRequestSchema,
  RemoveDependencyRequestSchema,
  DependencyCommandReceiptSchema,
} from "../contracts/dependency-command.ts";
import type { DependencyCommandAttempt } from "../contracts/dependency-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendDependencyCommand(
  fetcher: typeof fetch,
  attempt: DependencyCommandAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  if (attempt.action !== "create_dependency" && attempt.action !== "remove_dependency")
    throw new LiveCommandError("invalid_request");
  const schema =
    attempt.action === "create_dependency"
      ? CreateDependencyRequestSchema
      : RemoveDependencyRequestSchema;
  let body: unknown;
  try {
    body = JSON.parse(attempt.body) as unknown;
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const request = schema.safeParse(body);
  if (
    !request.success ||
    request.data.payload.blocker_task_id !== attempt.blockerId ||
    request.data.payload.blocked_task_id !== attempt.blockedId ||
    request.data.expected_revision !== attempt.expectedRevision
  )
    throw new LiveCommandError("invalid_request");
  const { status, value } = await postCommand(
    fetcher,
    attempt.action,
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = DependencyCommandReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.resource.id.toLowerCase() !== attempt.blockedId.toLowerCase() ||
    (receipt.data.event_ids.length === 0
      ? receipt.data.project_revision !== attempt.expectedRevision
      : receipt.data.project_revision <= attempt.expectedRevision)
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
