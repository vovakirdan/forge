import type { PipelineStageView, PipelineVersionView } from "../contracts/pipeline.ts";
import type { SystemStageAction } from "../contracts/stage-policy.ts";
import { pipelineStageLabel } from "../presentation/pipeline.ts";
import { Field } from "./Field.tsx";

export function PipelineStageInspector({
  stage,
  pipeline,
}: {
  stage: PipelineStageView;
  pipeline: PipelineVersionView;
}) {
  const workspace = stage.workspace;
  const acceptance = stage.acceptance_policy;
  return (
    <>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <Field label="Stage ID">{stage.id}</Field>
        <Field label="Stage name">{stage.name}</Field>
        <Field label="Executor">{stage.executor_kind}</Field>
        <Field label="Outcomes">
          {stage.outcomes.length ? stage.outcomes.join(", ") : "None listed"}
        </Field>
      </dl>
      <section aria-label="Stage instructions" className="space-y-2">
        <h4 className="font-medium">Instructions</h4>
        <p className="whitespace-pre-wrap text-sm [overflow-wrap:anywhere]">
          {stage.instructions || "No instructions provided."}
        </p>
      </section>
      <section aria-label="Workspace policy" className="space-y-2">
        <h4 className="font-medium">Workspace policy</h4>
        {workspace ? (
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
            <Field label="Workspace kind">{workspace.kind}</Field>
            <Field label="Workspace access">{workspace.access}</Field>
          </dl>
        ) : (
          <p className="text-sm">
            Not configured on this stage; a WorkSurface may still be provided.
          </p>
        )}
      </section>
      <section aria-label="Acceptance policy" className="space-y-2">
        <h4 className="font-medium">Acceptance policy</h4>
        {acceptance ? (
          <>
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
              <Field label="Acceptance kind">{acceptance.kind}</Field>
              <Field label="Independent review">{acceptance.independent ? "Yes" : "No"}</Field>
            </dl>
            <h5 className="text-sm font-medium">Outcome to verdict</h5>
            {Object.keys(acceptance.verdicts).length ? (
              <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm [overflow-wrap:anywhere]">
                {Object.entries(acceptance.verdicts).map(([outcome, verdict]) => (
                  <Field key={outcome} label={outcome}>
                    {verdict}
                  </Field>
                ))}
              </dl>
            ) : (
              <p className="text-sm">No verdict mappings listed.</p>
            )}
          </>
        ) : (
          <p className="text-sm">
            Not configured on this stage; this does not imply automatic acceptance.
          </p>
        )}
      </section>
      <section aria-label="System action" className="space-y-2">
        <h4 className="font-medium">System action</h4>
        <p className="text-xs text-muted-foreground">
          Read-only configuration. No action or hook is executed from this view.
        </p>
        {stage.system_action ? (
          <SystemAction action={stage.system_action} pipeline={pipeline} />
        ) : (
          <p className="text-sm">Not configured on this stage.</p>
        )}
      </section>
    </>
  );
}

function SystemAction({
  action,
  pipeline,
}: {
  action: SystemStageAction;
  pipeline: PipelineVersionView;
}) {
  switch (action.kind) {
    case "git_integration":
      return (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
          <Field label="Action kind">{action.kind}</Field>
          <Field label="Applied outcome">{action.outcomes.applied}</Field>
          <Field label="No changes outcome">{action.outcomes.no_changes}</Field>
          <Field label="Stale base outcome">{action.outcomes.stale_base}</Field>
          <Field label="Required review stages">
            {action.required_review_stages.length ? (
              <ul className="space-y-1">
                {action.required_review_stages.map((id) => (
                  <li key={id}>{pipelineStageLabel(pipeline, id)}</li>
                ))}
              </ul>
            ) : (
              "None configured"
            )}
          </Field>
        </dl>
      );
    case "project_hook":
      return (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
          <Field label="Action kind">{action.kind}</Field>
          <Field label="Hook version ID">{action.hook_version_id}</Field>
          <Field label="Passed outcome">{action.outcomes.passed}</Field>
          <Field label="Failed outcome">{action.outcomes.failed}</Field>
          <Field label="Timed out outcome">{action.outcomes.timed_out}</Field>
          <Field label="Skipped outcome">{action.outcomes.skipped}</Field>
        </dl>
      );
  }
}
