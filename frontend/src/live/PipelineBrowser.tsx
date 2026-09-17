import { useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError, LiveApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { PipelineCatalog } from "./PipelineCatalog.tsx";
import { PipelineDetailPanel } from "./PipelineDetailPanel.tsx";

export function PipelineBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const [versionId, setVersionId] = useState<string | null>(null);
  const opener = useRef<HTMLButtonElement | null>(null);
  const key = useMemo(
    () => readKeys.pipelineVersions(generation, projectId, navigation.cursor),
    [generation, projectId, navigation.cursor],
  );
  const versions = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.pipelines(projectId, navigation.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  function closeVersion() {
    setVersionId(null);
    if (opener.current?.isConnected) opener.current.focus();
  }
  function navigate(cursor: string | null, previous: (string | null)[]) {
    setVersionId(null);
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  const items = versions.data?.items;
  return (
    <>
      <section
        aria-label="Pipeline versions"
        className="space-y-4 rounded-xl border border-border bg-card p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Pipeline versions</h2>
          <Button
            variant="outline"
            disabled={versions.isFetching}
            onClick={() => void versions.refetch()}
          >
            Refresh pipeline versions
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          Read-only versions, up to 20 per page in server order. This page is not a complete catalog
          of Pipelines. Default and latest are independent.
        </p>
        {versions.isPending && <p role="status">Loading pipeline versions…</p>}
        {versions.isFetching && items && (
          <p role="status">Refreshing pipeline versions… Previous data remains visible.</p>
        )}
        {versions.isError && (
          <p role="alert">
            {items ? "Showing stale pipeline versions. " : ""}
            {describeApiError(versions.error, "Pipeline")}
          </p>
        )}
        {versions.error instanceof LiveApiError && versions.error.kind === "cursor_invalid" && (
          <Button
            variant="secondary"
            onClick={() => {
              if (navigation.cursor === null) void versions.refetch();
              else navigate(null, []);
            }}
          >
            Restart pagination
          </Button>
        )}
        {items?.length === 0 && (
          <p>
            {navigation.cursor === null
              ? "No pipeline versions in this project."
              : "No pipeline versions on this page."}
          </p>
        )}
        {items && items.length > 0 && (
          <ul aria-label="Pipeline version list" className="space-y-3">
            {items.map((pipeline) => (
              <li key={pipeline.id} className="space-y-3 rounded-lg border border-border p-4">
                <button
                  type="button"
                  aria-label={`Open Pipeline version ${pipeline.id}`}
                  aria-controls="pipeline-version-details"
                  onClick={(event) => {
                    opener.current = event.currentTarget;
                    if (versionId === pipeline.id)
                      document.getElementById("pipeline-version-details-title")?.focus();
                    else setVersionId(pipeline.id);
                  }}
                  className="w-full cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
                >
                  {pipeline.name} · version {pipeline.version}
                </button>
                <PipelineCatalog pipeline={pipeline} />
              </li>
            ))}
          </ul>
        )}
        <nav
          aria-label="Pipeline version pagination"
          className="flex flex-wrap items-center justify-between gap-3"
        >
          <Button
            variant="outline"
            disabled={versions.isFetching || navigation.previous.length === 0}
            onClick={() =>
              navigate(navigation.previous.at(-1) ?? null, navigation.previous.slice(0, -1))
            }
          >
            Previous page
          </Button>
          <p className="text-sm">Page {navigation.previous.length + 1}</p>
          <Button
            variant="outline"
            disabled={
              versions.isFetching || versions.isError || versions.data?.next_cursor === undefined
            }
            onClick={() => {
              const next = versions.data?.next_cursor;
              if (next !== undefined) navigate(next, [...navigation.previous, navigation.cursor]);
            }}
          >
            Next page
          </Button>
        </nav>
      </section>
      {versionId && (
        <PipelineDetailPanel
          key={versionId}
          {...scope}
          versionId={versionId}
          onClose={closeVersion}
        />
      )}
    </>
  );
}
