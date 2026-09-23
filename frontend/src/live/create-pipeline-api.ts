import {
  createPipelineAttempt,
  CreatePipelineReceiptSchema,
  type CreatePipelineAttempt,
} from "../contracts/create-pipeline.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendCreatePipeline(
  fetcher: typeof fetch,
  attempt: CreatePipelineAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const request = JSON.parse(attempt.body) as {
      project_id: string;
      expected_revision: number;
      payload: { name: string; [key: string]: unknown };
    };
    const { name, ...definition } = request.payload;
    const checked = createPipelineAttempt(
      request.project_id,
      request.expected_revision,
      name,
      definition,
      attempt.key,
    );
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.expectedProjectRevision !== attempt.expectedProjectRevision ||
      checked.name !== attempt.name
    )
      throw new Error("attempt changed");
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const { status, value } = await postCommand(
    fetcher,
    "create_pipeline",
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = CreatePipelineReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedProjectRevision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
