import { useEffect, useMemo, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { PipelineCatalog } from "./PipelineCatalog.tsx";
import { PipelineDefinition } from "./PipelineDefinition.tsx";

export function PipelineDetailPanel({
  api,
  session,
  generation,
  projectId,
  versionId,
  onClose,
}: ProjectReadScope & { versionId: string; onClose: () => void }) {
  const title = useRef<HTMLHeadingElement>(null);
  const key = useMemo(
    () => readKeys.pipelineVersion(generation, projectId, versionId),
    [generation, projectId, versionId],
  );
  const detail = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.pipeline(projectId, versionId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  useEffect(() => {
    title.current?.focus();
  }, []);
  const pipeline = detail.data;
  return (
    <section
      id="pipeline-version-details"
      aria-label="Pipeline version details"
      className="min-w-0 space-y-5 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2
          id="pipeline-version-details-title"
          ref={title}
          tabIndex={-1}
          className="rounded text-lg font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Pipeline version details
        </h2>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            disabled={detail.isFetching}
            onClick={() => void detail.refetch()}
          >
            Refresh pipeline version
          </Button>
          <Button variant="outline" onClick={onClose}>
            Close pipeline version
          </Button>
        </div>
      </div>
      {detail.isPending && <p role="status">Loading Pipeline version details…</p>}
      {detail.isFetching && pipeline && (
        <p role="status">Refreshing Pipeline version details… Previous data remains visible.</p>
      )}
      {detail.isError && (
        <p role="alert">
          {pipeline ? "Showing stale Pipeline version details. " : ""}
          {describeApiError(detail.error, "Pipeline")}
        </p>
      )}
      {pipeline && (
        <>
          <section aria-label="Pipeline catalog" className="space-y-3">
            <h3 className="font-medium">Current catalog</h3>
            <p className="text-xs text-muted-foreground">
              Name, revision, default/latest and deletion state can change. These are the values
              from this read, not immutable version metadata. Soft deletion preserves version
              history.
            </p>
            <PipelineCatalog pipeline={pipeline} detail />
          </section>
          <PipelineDefinition key={pipeline.id} pipeline={pipeline} />
        </>
      )}
    </section>
  );
}
