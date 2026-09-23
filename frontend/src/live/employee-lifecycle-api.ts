import {
  EmployeeCommandReceiptSchema,
  EmployeeLifecycleRequestSchema,
  type EmployeeLifecycleAttempt,
} from "../contracts/employee-lifecycle.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendEmployeeLifecycle(
  fetcher: typeof fetch,
  attempt: EmployeeLifecycleAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = EmployeeLifecycleRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  const { status, value } = await postCommand(
    fetcher,
    attempt.action,
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = EmployeeCommandReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.resource.id.toLowerCase() !== request.payload.employee_id.toLowerCase() ||
    receipt.data.project_revision !== request.expected_revision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
