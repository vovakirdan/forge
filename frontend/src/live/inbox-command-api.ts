import {
  inboxAttempt,
  InboxReceiptSchema,
  type InboxAttempt,
} from "../contracts/communication-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

const expectedKind = {
  open_employee_thread: "employee_thread",
  send_employee_message: "employee_message",
  waive_message_requirement: "message_requirement_waiver",
  retry_communication: "run",
} as const;
export async function sendInboxCommand(
  fetcher: typeof fetch,
  attempt: InboxAttempt,
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
    const checked = inboxAttempt(attempt.action, request, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.expectedRevision !== attempt.expectedRevision ||
      checked.resourceId !== attempt.resourceId
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
  const receipt = InboxReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedRevision + 1 ||
    receipt.data.resource.kind !== expectedKind[attempt.action] ||
    (attempt.resourceId !== null &&
      receipt.data.resource.id.toLowerCase() !== attempt.resourceId.toLowerCase())
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
