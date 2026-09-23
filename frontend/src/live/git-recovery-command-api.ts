import {
  GitRecoveryReceiptSchema,
  gitRecoveryAttempt,
  type GitRecoveryAttempt,
} from "../contracts/git-recovery-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendGitRecoveryCommand(
  fetcher: typeof fetch,
  attempt: GitRecoveryAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const request: unknown = JSON.parse(attempt.body);
    const checked = gitRecoveryAttempt(
      attempt.action,
      request as Parameters<typeof gitRecoveryAttempt>[1],
      attempt.key,
    );
    if (
      checked.body !== attempt.body ||
      checked.operationId !== attempt.operationId ||
      checked.projectId !== attempt.projectId ||
      checked.expectedTaskRevision !== attempt.expectedTaskRevision
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
  const receipt = GitRecoveryReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedRevision + 1 ||
    receipt.data.resource.id.toLowerCase() !== attempt.operationId.toLowerCase()
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
