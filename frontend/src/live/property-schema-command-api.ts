import {
  ConfigureTaskPropertySchemaRequest,
  PropertySchemaReceipt,
  createPropertySchemaAttempt,
  type PropertySchemaAttempt,
} from "../contracts/property-schema-command.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendPropertySchemaCommand(
  fetcher: typeof fetch,
  attempt: PropertySchemaAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  let request: ReturnType<typeof ConfigureTaskPropertySchemaRequest.parse>;
  try {
    request = ConfigureTaskPropertySchemaRequest.parse(JSON.parse(attempt.body) as unknown);
    const checked = createPropertySchemaAttempt(
      request.project_id,
      request.expected_revision,
      request.payload.schema,
      attempt.key,
    );
    if (checked.body !== attempt.body || checked.projectId !== attempt.projectId)
      throw new Error("attempt changed");
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const { status, value } = await postCommand(
    fetcher,
    "configure_task_property_schema",
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = PropertySchemaReceipt.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.resource.id.toLowerCase() !== request.project_id.toLowerCase() ||
    receipt.data.project_revision !== request.expected_revision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
