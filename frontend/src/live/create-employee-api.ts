import {
  CreateEmployeeRequestSchema,
  EmployeeCommandReceiptSchema,
  type CreateEmployeeAttempt,
} from "../contracts/create-employee.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendCreateEmployee(
  fetcher: typeof fetch,
  attempt: CreateEmployeeAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  const request = CreateEmployeeRequestSchema.parse(JSON.parse(attempt.body) as unknown);
  const { status, value } = await postCommand(
    fetcher,
    "create_employee",
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = EmployeeCommandReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== request.expected_revision + 1
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
