import { useState } from "react";
import type { PipelineVersionView } from "../contracts/pipeline.ts";
import { pipelineStageLabel, pipelineTargetLabel } from "../presentation/pipeline.ts";
import { Field } from "./Field.tsx";
import { PipelineStageInspector } from "./PipelineStageInspector.tsx";

export function PipelineDefinition({ pipeline }: { pipeline: PipelineVersionView }) {
  const [stageId, setStageId] = useState(pipeline.entry_stage_id);
  const selectedStage = pipeline.stages.find((stage) => stage.id === stageId);
  return (
    <section aria-label="Version definition" className="min-w-0 space-y-5">
      <h3 className="font-medium">Immutable version definition</h3>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <Field label="Task kinds">
          {pipeline.task_kinds.length ? pipeline.task_kinds.join(", ") : "None listed"}
        </Field>
        <Field label="Entry stage">{pipelineStageLabel(pipeline, pipeline.entry_stage_id)}</Field>
        <Field label="Max stage visits">
          {pipeline.max_stage_visits ??
            "Not configured; this does not promise unlimited execution."}
        </Field>
      </dl>
      <div className="min-w-0 overflow-x-auto rounded border border-border">
        <table
          aria-label="Stages"
          className="w-full table-fixed text-left text-sm [overflow-wrap:anywhere]"
        >
          <caption className="p-3 text-left font-medium">Stages</caption>
          <thead>
            <tr className="border-b border-border">
              <th scope="col" className="p-3">
                Stage
              </th>
              <th scope="col" className="p-3">
                Executor
              </th>
              <th scope="col" className="p-3">
                Outcomes
              </th>
            </tr>
          </thead>
          <tbody>
            {pipeline.stages.map((stage) => (
              <tr key={stage.id} className="border-b border-border last:border-0">
                <th scope="row" className="p-3 font-normal">
                  <button
                    type="button"
                    aria-label={`Inspect stage ${stage.id}`}
                    aria-pressed={stage.id === stageId}
                    aria-controls="selected-pipeline-stage"
                    onClick={() => setStageId(stage.id)}
                    className="cursor-pointer rounded text-left text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    {stage.name} ({stage.id})
                  </button>
                </th>
                <td className="p-3">{stage.executor_kind}</td>
                <td className="p-3">
                  {stage.outcomes.length ? stage.outcomes.join(", ") : "None listed"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {pipeline.stages.length === 0 && <p className="p-3 text-sm">No stages listed.</p>}
      </div>
      <div className="min-w-0 overflow-x-auto rounded border border-border">
        <table
          aria-label="Transitions"
          className="w-full table-fixed text-left text-sm [overflow-wrap:anywhere]"
        >
          <caption className="p-3 text-left font-medium">Transitions</caption>
          <thead>
            <tr className="border-b border-border">
              <th scope="col" className="p-3">
                From stage
              </th>
              <th scope="col" className="p-3">
                Outcome
              </th>
              <th scope="col" className="p-3">
                Target
              </th>
              <th scope="col" className="p-3">
                Artifact requirements
              </th>
            </tr>
          </thead>
          <tbody>
            {pipeline.transitions.map((transition, index) => (
              <tr
                key={`${transition.from_stage_id}:${transition.outcome}:${index}`}
                className="border-b border-border last:border-0"
              >
                <th scope="row" className="p-3 font-normal">
                  {pipelineStageLabel(pipeline, transition.from_stage_id)}
                </th>
                <td className="p-3">{transition.outcome}</td>
                <td className="p-3">{pipelineTargetLabel(pipeline, transition.target)}</td>
                <td className="p-3">
                  {transition.artifact_requirements?.length ? (
                    <ul className="space-y-2">
                      {transition.artifact_requirements.map((artifact, artifactIndex) => (
                        <li key={`${artifact.kind}:${artifact.scope}:${artifactIndex}`}>
                          {artifact.kind}; minimum count: {artifact.minimum_count}; scope:{" "}
                          {artifact.scope}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    "None configured on this transition"
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {pipeline.transitions.length === 0 && <p className="p-3 text-sm">No transitions listed.</p>}
      </div>
      <section
        id="selected-pipeline-stage"
        aria-label="Selected stage"
        className="space-y-3 rounded border border-border p-4"
      >
        <h3 className="font-medium">Selected stage</h3>
        {selectedStage ? (
          <PipelineStageInspector stage={selectedStage} pipeline={pipeline} />
        ) : (
          <p className="text-sm [overflow-wrap:anywhere]">
            {pipelineStageLabel(pipeline, stageId)}
          </p>
        )}
      </section>
    </section>
  );
}
