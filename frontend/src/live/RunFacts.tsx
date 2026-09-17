import type { RunView } from "../contracts/run.ts";
import type { ExecutionAssignment } from "../contracts/execution-assignment.ts";
import { presentRun } from "../presentation/run.ts";
import { Field } from "./Field.tsx";

export function RunFacts({ run, detail = false }: { run: RunView; detail?: boolean }) {
  const { presentation } = presentRun(run);
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
      <Field label="Run ID">{run.id}</Field>
      <Field label="Purpose">
        {presentation.purposeLabel} ({run.assignment.purpose})
      </Field>
      <Field label="Task ID">{run.task_id ?? "Not assigned"}</Field>
      <Field label="Employee ID">{run.employee_id ?? "Not assigned"}</Field>
      <Field label="Stage ID">{run.stage_id ?? "Not assigned"}</Field>
      <Field label="Attempt">{run.attempt}</Field>
      <Field label="Desired state">
        {presentation.desiredStateLabel} ({run.desired_state})
      </Field>
      <Field label="Observed state">
        {presentation.observedStateLabel} ({run.observed_state})
      </Field>
      {detail && (
        <>
          <Field label="Lease fencing token">{run.lease_fencing_token}</Field>
          <Field label="Environment epoch">{run.environment_epoch}</Field>
          <Field label="Last observed sequence">{run.last_observed_sequence}</Field>
          <Field label="RunSpec version">{run.run_spec_version}</Field>
        </>
      )}
    </dl>
  );
}

export function RunOwner({ run }: { run: RunView }) {
  return (
    <section aria-label="Assignment owner" className="space-y-2">
      <h3 className="font-medium">Assignment owner</h3>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <OwnerFields assignment={run.assignment} />
        {run.assignment.purpose === "system_job" && (
          <Field label="System job kind">
            {presentRun(run).presentation.systemJobKindLabel} ({run.assignment.owner.kind})
          </Field>
        )}
      </dl>
    </section>
  );
}

function OwnerFields({ assignment }: { assignment: ExecutionAssignment }) {
  switch (assignment.purpose) {
    case "task_stage":
      return (
        <>
          <Field label="Owner Task ID">{assignment.owner.task_id}</Field>
          <Field label="Queue entry ID">{assignment.owner.queue_entry_id}</Field>
          <Field label="Owner stage ID">{assignment.owner.stage_id}</Field>
        </>
      );
    case "communication":
      return (
        <>
          <Field label="Assignment ID">{assignment.owner.assignment_id}</Field>
          <Field label="Thread ID">{assignment.owner.thread_id}</Field>
          <Field label="Source message ID">{assignment.owner.source_message_id}</Field>
        </>
      );
    case "resolution":
      return (
        <>
          <Field label="Assignment ID">{assignment.owner.assignment_id}</Field>
          <Field label="Escalation ID">{assignment.owner.escalation_id}</Field>
          <Field label="Lease generation">{assignment.owner.lease_generation}</Field>
        </>
      );
    case "hook":
      return (
        <>
          <Field label="Invocation ID">{assignment.owner.invocation_id}</Field>
          <Field label="Context Task ID">{assignment.owner.task_id}</Field>
          <Field label="Pipeline version ID">{assignment.owner.pipeline_version_id}</Field>
          <Field label="Context stage ID">{assignment.owner.stage_id}</Field>
          <Field label="Stage visit">{assignment.owner.stage_visit}</Field>
          <Field label="Candidate proposal ID">{assignment.owner.candidate_proposal_id}</Field>
        </>
      );
    case "system_job":
      return (
        <>
          <Field label="Job ID">{assignment.owner.job_id}</Field>
          <Field label="Job attempt ID">{assignment.owner.attempt_id}</Field>
          <Field label="Generation">{assignment.owner.generation}</Field>
        </>
      );
    default: {
      const unreachable: never = assignment;
      return unreachable;
    }
  }
}
