import type { PipelineTargetView, PipelineVersionView } from "../contracts/pipeline.ts";

/** Catalog flags are current read facts, not immutable properties of the graph. */
export function presentPipelineVersion(pipeline: PipelineVersionView) {
  return {
    source: pipeline,
    presentation: {
      isDefault: pipeline.id.toLowerCase() === pipeline.default_version_id.toLowerCase(),
      isLatest: pipeline.version === pipeline.latest_version,
      deletionLabel: pipeline.deleted_at === null ? "Not deleted" : "Soft deleted",
    },
  };
}

export function pipelineStageLabel(pipeline: PipelineVersionView, stageId: string): string {
  const stage = pipeline.stages.find((item) => item.id === stageId);
  return stage ? `${stage.name} (${stage.id})` : `${stageId} — stage unavailable`;
}

export function pipelineTargetLabel(
  pipeline: PipelineVersionView,
  target: PipelineTargetView,
): string {
  switch (target.kind) {
    case "stage":
      return pipelineStageLabel(pipeline, target.stage_id);
    case "done":
      return "done (terminal)";
    case "cancelled":
      return "cancelled (terminal)";
  }
}
