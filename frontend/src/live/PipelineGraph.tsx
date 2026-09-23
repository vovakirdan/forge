import type { PipelineVersionView } from "../contracts/pipeline.ts";
import { pipelineStageLabel, pipelineTargetLabel } from "../presentation/pipeline.ts";

export function PipelineGraph({
  pipeline,
  selectedStageId,
  onSelectStage,
}: {
  pipeline: PipelineVersionView;
  selectedStageId: string;
  onSelectStage: (id: string) => void;
}) {
  return (
    <section aria-label="Pipeline graph" className="space-y-3">
      <div>
        <h3 className="font-medium">Stage graph</h3>
        <p className="text-xs text-muted-foreground">
          Each card shows its outcome routes. Arrows to another stage can form cycles; done and
          cancelled end the route. Select a stage to inspect its contract.
        </p>
      </div>
      <ol className="grid gap-3 md:grid-cols-2">
        {pipeline.stages.map((stage) => {
          const routes = pipeline.transitions.filter(
            (transition) => transition.from_stage_id === stage.id,
          );
          return (
            <li
              key={stage.id}
              className={`min-w-0 rounded border p-3 text-sm [overflow-wrap:anywhere] ${
                selectedStageId === stage.id ? "border-primary bg-primary/5" : "border-border"
              }`}
            >
              <div className="flex flex-wrap items-start justify-between gap-2">
                <button
                  type="button"
                  aria-label={`View graph stage ${stage.id}`}
                  aria-pressed={selectedStageId === stage.id}
                  aria-controls="selected-pipeline-stage"
                  onClick={() => onSelectStage(stage.id)}
                  className="cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {stage.name} ({stage.id})
                </button>
                {stage.id === pipeline.entry_stage_id && (
                  <span className="rounded bg-muted px-2 py-0.5 text-xs">Entry stage</span>
                )}
              </div>
              <p className="mt-1 text-xs text-muted-foreground">Executor: {stage.executor_kind}</p>
              {routes.length ? (
                <ul className="mt-3 space-y-2 border-t border-border pt-2">
                  {routes.map((route, index) => {
                    const target = route.target;
                    return (
                      <li key={`${route.outcome}:${index}`}>
                        <span className="font-medium">{route.outcome}</span>
                        <span aria-hidden="true"> → </span>
                        {target.kind === "stage" ? (
                          <button
                            type="button"
                            aria-label={`Follow ${route.outcome} to stage ${target.stage_id}`}
                            onClick={() => onSelectStage(target.stage_id)}
                            className="cursor-pointer rounded text-left text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                          >
                            {pipelineStageLabel(pipeline, target.stage_id)}
                          </button>
                        ) : (
                          <span>{pipelineTargetLabel(pipeline, target)}</span>
                        )}
                      </li>
                    );
                  })}
                </ul>
              ) : (
                <p className="mt-3 border-t border-border pt-2 text-muted-foreground">
                  No outgoing routes listed.
                </p>
              )}
            </li>
          );
        })}
      </ol>
      {pipeline.stages.length === 0 && <p className="text-sm">No stages listed.</p>}
    </section>
  );
}
