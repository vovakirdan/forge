import { LiveCommandError } from "./command-error.ts";

export function describeApprovalError(error: unknown) {
  const kind = error instanceof LiveCommandError ? error.kind : "outcome_unknown";
  switch (kind) {
    case "outcome_unknown":
      return "Approval outcome unknown. Core may have approved the Task. Retry the same approval to obtain its receipt; do not submit a new approval.";
    case "stale_revision":
      return "Not approved. The Project or Task changed. Refresh the approval baseline and confirm again.";
    case "idempotency_conflict":
      return "Approval refused: its key belongs to another request. Refresh the approval baseline.";
    case "not_found":
      return "Not approved. The Project or Task was not found. Refresh the approval baseline.";
    case "forbidden":
      return "Not approved. The gateway refused this command. Refresh the approval baseline.";
    case "invalid_request":
    case "validation_failed":
      return "Not approved. Core rejected approval. Check the saved DoD, required properties, WorkSurface and pinned Pipeline requirements. Refresh the approval baseline before trying again.";
  }
}
