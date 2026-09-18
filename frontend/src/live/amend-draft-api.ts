import { AmendDraftRequestSchema } from "../contracts/amend-draft.ts";
import type { DraftAttempt } from "./draft-attempt.ts";
import { sendTaskCommand } from "./task-command-api.ts";

export async function sendAmendDraft(
  fetcher: typeof fetch,
  attempt: DraftAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = AmendDraftRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  return sendTaskCommand(fetcher, "amend_draft", attempt, request, token, signal, unauthorized);
}
