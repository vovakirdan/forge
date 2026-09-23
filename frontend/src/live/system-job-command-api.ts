import {
  systemJobAttempt,
  SystemJobReceiptSchema,
  type SystemJobAttempt,
} from "../contracts/system-job-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendSystemJobCommand(
  fetcher: typeof fetch,
  attempt: SystemJobAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const request = JSON.parse(attempt.body) as {
      project_id: string;
      expected_revision: number;
      payload: unknown;
    };
    const checked = systemJobAttempt(attempt.action, request, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.expectedProjectRevision !== attempt.expectedProjectRevision ||
      checked.resourceKind !== attempt.resourceKind ||
      checked.expectedResourceId !== attempt.expectedResourceId
    )
      throw new Error("attempt changed");
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
  const receipt = SystemJobReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedProjectRevision + 1 ||
    (attempt.resourceKind === null
      ? receipt.data.resource !== undefined
      : receipt.data.resource?.kind !== attempt.resourceKind ||
        (attempt.expectedResourceId !== null &&
          receipt.data.resource.id.toLowerCase() !== attempt.expectedResourceId.toLowerCase()))
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
