import {
  pipelineManagementAttempt,
  PipelineManagementReceiptSchema,
  type PipelineManagementAttempt,
} from "../contracts/pipeline-management.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendPipelineManagementCommand(
  fetcher: typeof fetch,
  attempt: PipelineManagementAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const parsed = JSON.parse(attempt.body) as {
      project_id: string;
      expected_revision: number;
      payload: unknown;
    };
    const checked = pipelineManagementAttempt(attempt.action, parsed, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.pipelineId !== attempt.pipelineId ||
      checked.expectedProjectRevision !== attempt.expectedProjectRevision
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
  const receipt = PipelineManagementReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.resource.kind !== attempt.resourceKind ||
    (attempt.resourceKind === "pipeline" &&
      receipt.data.resource.id.toLowerCase() !== attempt.pipelineId.toLowerCase()) ||
    receipt.data.project_revision !== attempt.expectedProjectRevision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
