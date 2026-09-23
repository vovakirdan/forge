export type CommandErrorKind =
  | "invalid_request"
  | "stale_revision"
  | "conflict"
  | "idempotency_conflict"
  | "validation_failed"
  | "not_found"
  | "forbidden"
  | "outcome_unknown";

export class LiveCommandError extends Error {
  readonly kind: CommandErrorKind;
  constructor(kind: CommandErrorKind) {
    super(kind);
    this.name = "LiveCommandError";
    this.kind = kind;
  }
}

export function describeCommandError(error: unknown) {
  const kind = error instanceof LiveCommandError ? error.kind : "outcome_unknown";
  switch (kind) {
    case "stale_revision":
      return "Not saved. The Project changed. Refresh the edit baseline before saving again.";
    case "conflict":
      return "Not saved. Core refused this action in the current state. Refresh and review it.";
    case "invalid_request":
      return "Not saved. The Task may have changed or this edit is no longer allowed. Refresh the edit baseline.";
    case "idempotency_conflict":
      return "This save was refused because its key belongs to another request. Refresh the edit baseline.";
    case "validation_failed":
      return "Not saved. Core rejected the edit. Refresh the edit baseline and check the fields.";
    case "not_found":
      return "Not saved. The Project or Task was not found. Refresh the edit baseline.";
    case "forbidden":
      return "Not saved. This edit was refused by the gateway. Refresh the edit baseline before trying again.";
    case "outcome_unknown":
      return "Save outcome unknown. Core may have saved the edit. Retry the same save to obtain its receipt; do not submit a new edit.";
  }
}
