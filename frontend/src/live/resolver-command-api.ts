import {
  resolverAttempt,
  ResolverReceiptSchema,
  type ResolverAttempt,
} from "../contracts/resolver-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendResolverCommand(
  fetcher: typeof fetch,
  attempt: ResolverAttempt,
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
    const checked = resolverAttempt(attempt.action, parsed, attempt.key);
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.expectedRevision !== attempt.expectedRevision ||
      checked.routeKey !== attempt.routeKey ||
      checked.taskId !== attempt.taskId
    )
      throw Error("attempt changed");
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
  const receipt = ResolverReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision <= attempt.expectedRevision ||
    receipt.data.resource.kind !==
      (attempt.action === "configure_resolver_route" ? "project" : "escalation") ||
    (attempt.action === "configure_resolver_route" &&
      receipt.data.resource.id.toLowerCase() !== attempt.projectId.toLowerCase())
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
