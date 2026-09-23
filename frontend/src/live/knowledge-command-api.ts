import {
  KnowledgeReceiptSchema,
  knowledgeAttempt,
  type KnowledgeAttempt,
} from "../contracts/knowledge.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendKnowledgeCommand(
  fetcher: typeof fetch,
  attempt: KnowledgeAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  let request: { expected_revision: number };
  try {
    const parsed = JSON.parse(attempt.body) as {
      project_id: string;
      expected_revision: number;
      payload: unknown;
    };
    const checked = knowledgeAttempt(attempt.action, parsed, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.pageId.toLowerCase() !== attempt.pageId.toLowerCase()
    )
      throw new Error("attempt changed");
    request = parsed;
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
  const receipt = KnowledgeReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.resource.id.toLowerCase() !== attempt.pageId.toLowerCase() ||
    receipt.data.project_revision !== request.expected_revision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
