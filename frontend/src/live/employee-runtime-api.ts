import {
  configureEmployeeRuntimeAttempt,
  EmployeeRuntimeReceiptSchema,
  type EmployeeRuntimeAttempt,
} from "../contracts/employee-runtime.ts";
import { LiveCommandError } from "./command-error.ts";
import { postCommand } from "./task-command-api.ts";

export async function sendConfigureEmployeeRuntime(
  fetcher: typeof fetch,
  attempt: EmployeeRuntimeAttempt,
  token: string,
  signal: AbortSignal,
  unauthorized: () => Error,
) {
  try {
    const request = JSON.parse(attempt.body) as {
      project_id: string;
      expected_revision: number;
      payload: { employee_id: string; binding: unknown };
    };
    const checked = configureEmployeeRuntimeAttempt(
      request.project_id,
      request.payload.employee_id,
      request.expected_revision,
      request.payload.binding,
      attempt.key,
    );
    if (
      checked.body !== attempt.body ||
      checked.projectId !== attempt.projectId ||
      checked.employeeId !== attempt.employeeId ||
      checked.expectedProjectRevision !== attempt.expectedProjectRevision
    )
      throw new Error("attempt changed");
  } catch {
    throw new LiveCommandError("invalid_request");
  }
  const { status, value } = await postCommand(
    fetcher,
    "configure_employee_runtime",
    attempt,
    token,
    signal,
    unauthorized,
  );
  const receipt = EmployeeRuntimeReceiptSchema.safeParse(value);
  if (
    status !== 200 ||
    !receipt.success ||
    receipt.data.project_revision !== attempt.expectedProjectRevision + 1 ||
    receipt.data.resource.id.toLowerCase() !== attempt.employeeId.toLowerCase()
  )
    throw new LiveCommandError("outcome_unknown");
  return receipt.data;
}
