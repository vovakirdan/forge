import type { ExecutionAssignment, SystemJobKind } from "../contracts/execution-assignment.ts";
import type { RunDesiredState, RunObservedState, RunView } from "../contracts/run.ts";

const purposeLabels = {
  task_stage: "Task stage",
  communication: "Communication",
  resolution: "Resolution",
  hook: "Hook",
  system_job: "System job",
} as const satisfies Record<ExecutionAssignment["purpose"], string>;

const systemJobKindLabels = {
  summarization: "Summarization",
  onboarding: "Onboarding",
} as const satisfies Record<SystemJobKind, string>;

const desiredStateLabels = {
  provision_requested: "Provision requested",
  running: "Running",
  stop_requested: "Stop requested",
  force_stop_requested: "Force stop requested",
  stopped: "Stopped",
  failed: "Failed",
} as const satisfies Record<RunDesiredState, string>;

const observedStateLabels = {
  unknown: "Unknown",
  provisioning: "Provisioning",
  running: "Running",
  stopping: "Stopping",
  stopped: "Stopped",
  failed: "Failed",
  lost: "Lost",
} as const satisfies Record<RunObservedState, string>;

/** Requested state, observed state and assignment remain independent facts. */
export function presentRun<TRun extends RunView>(run: TRun) {
  return {
    source: run,
    presentation: {
      purposeLabel: purposeLabels[run.assignment.purpose],
      desiredStateLabel: desiredStateLabels[run.desired_state],
      observedStateLabel: observedStateLabels[run.observed_state],
      systemJobKindLabel:
        run.assignment.purpose === "system_job"
          ? systemJobKindLabels[run.assignment.owner.kind]
          : null,
    },
  };
}

export type PresentedRun = ReturnType<typeof presentRun>;
