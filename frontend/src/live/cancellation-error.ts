import { LiveCommandError } from "./command-error.ts";

export function describeCancellationError(error: unknown) {
  const kind = error instanceof LiveCommandError ? error.kind : "outcome_unknown";
  switch (kind) {
    case "outcome_unknown":
      return "Cancellation outcome unknown. Core may have cancelled the Task. Retry the same cancellation to obtain its receipt; do not submit a new cancellation.";
    case "stale_revision":
      return "Not cancelled. The Project or Task changed. Refresh the cancellation baseline and confirm again.";
    case "idempotency_conflict":
      return "Cancellation refused: its key belongs to another request. Refresh the cancellation baseline.";
    case "not_found":
      return "Not cancelled. The Project or Task was not found. Refresh the cancellation baseline.";
    case "forbidden":
      return "Not cancelled. The gateway refused this command. Refresh the cancellation baseline.";
    case "invalid_request":
    case "validation_failed":
      return "Not cancelled. Core rejected cancellation. Check that the Task is nonterminal and the selected reason is still active. Refresh the cancellation baseline before trying again.";
  }
}
