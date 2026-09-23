import {
  configureProjectHookAttempt,
  ProjectHookReceiptSchema,
  type ProjectHookAttempt,
} from "../contracts/project-hook.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendConfigureProjectHook(
  fetcher: typeof fetch,
  attempt: ProjectHookAttempt,
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
    const checked = configureProjectHookAttempt(
      request.project_id,
      request.expected_revision,
      request.payload,
      attempt.key,
    );
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.expectedProjectRevision !== attempt.expectedProjectRevision
    )
      throw new Error("attempt changed");
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const { status, value } = await postCommand(
    fetcher,
    "configure_project_hook",
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = ProjectHookReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedProjectRevision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
