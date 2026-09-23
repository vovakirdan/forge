import {
  createProjectAttempt,
  CreateProjectReceiptSchema,
  type CreateProjectAttempt,
} from "../contracts/create-project.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendCreateProject(
  fetcher: typeof fetch,
  attempt: CreateProjectAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const request = JSON.parse(attempt.body) as { project_id: string; payload: { name: string } };
    const checked = createProjectAttempt(request.project_id, request.payload.name, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.name !== attempt.name
    )
      throw new Error("attempt changed");
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const { status, value } = await postCommand(
    fetcher,
    "create_project",
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = CreateProjectReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.resource.id.toLowerCase() !== attempt.projectId.toLowerCase()
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
