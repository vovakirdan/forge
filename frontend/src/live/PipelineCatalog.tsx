import type { PipelineVersionView } from "../contracts/pipeline.ts";
import { presentPipelineVersion } from "../presentation/pipeline.ts";
import { Field } from "./Field.tsx";

export function PipelineCatalog({
  pipeline,
  detail = false,
}: {
  pipeline: PipelineVersionView;
  detail?: boolean;
}) {
  const { isDefault, isLatest, deletionLabel } = presentPipelineVersion(pipeline).presentation;
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
      <Field label="Name">{pipeline.name}</Field>
      <Field label="Pipeline ID">{pipeline.pipeline_id}</Field>
      <Field label="Version ID">{pipeline.id}</Field>
      <Field label="Version">{pipeline.version}</Field>
      <Field label="Default version">{isDefault ? "Yes" : "No"}</Field>
      <Field label="Latest version">{isLatest ? "Yes" : "No"}</Field>
      <Field label="Deletion state">{deletionLabel}</Field>
      {detail && (
        <>
          <Field label="Catalog revision">{pipeline.catalog_revision}</Field>
          <Field label="Default version ID">{pipeline.default_version_id}</Field>
          <Field label="Latest version number">{pipeline.latest_version}</Field>
          <Field label="Deleted at">{pipeline.deleted_at ?? "Not deleted"}</Field>
        </>
      )}
    </dl>
  );
}
