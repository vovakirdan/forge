import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { PipelineCatalogActions } from "./PipelineCatalogActions.tsx";
import { PipelineCreatePanel } from "./PipelineCreatePanel.tsx";
import { ProjectHooksBrowser } from "./ProjectHooksBrowser.tsx";

export function PipelineCatalogBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [managing, setManaging] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [createdId, setCreatedId] = useState<string | null>(null);
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const key = useMemo(
    () => readKeys.pipelineCatalog(generation, projectId, navigation.cursor),
    [generation, projectId, navigation.cursor],
  );
  const catalog = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.pipelineCatalog(projectId, navigation.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  function navigate(cursor: string | null, previous: (string | null)[]) {
    if ((managing || creating) && !scope.leaveGuard.canLeave()) return;
    setManaging(null);
    setCreating(false);
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  return (
    <>
      <section
        aria-label="Pipeline catalog"
        className="space-y-4 rounded-xl border border-border bg-card p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Pipeline catalog</h2>
          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={() => {
                if ((managing || creating) && !scope.leaveGuard.canLeave()) return;
                setManaging(null);
                setCreating(!creating);
              }}
            >
              {creating ? "Hide creation" : "Create Pipeline"}
            </Button>
            <Button
              variant="outline"
              disabled={catalog.isFetching}
              onClick={() => void catalog.refetch()}
            >
              Refresh catalog
            </Button>
          </div>
        </div>
        {creating && (
          <PipelineCreatePanel
            {...scope}
            onClose={() => setCreating(false)}
            onApplied={(id) => {
              setCreatedId(id);
              setCreating(false);
              void catalog.refetch();
            }}
          />
        )}
        {createdId && (
          <p role="status" className="text-sm">
            Created Pipeline <span className="font-mono">{createdId}</span>. It may appear on a
            later catalog page.
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          One row per Pipeline, up to 20 per page. Default affects future Tasks; existing Tasks keep
          their pinned version. Deleted Pipelines remain in history.
        </p>
        {catalog.isPending && <p role="status">Loading Pipeline catalog…</p>}
        {catalog.isError && (
          <p role="alert">
            {catalog.data ? "Showing stale Pipeline catalog. " : ""}
            {describeApiError(catalog.error, "Pipeline")}
          </p>
        )}
        {catalog.data?.items.length === 0 && <p>No Pipelines on this page.</p>}
        {catalog.data && (
          <ul aria-label="Pipeline catalog entries" className="space-y-2">
            {catalog.data.items.map((item) => (
              <li
                key={item.id}
                className="rounded border border-border p-3 text-sm [overflow-wrap:anywhere]"
              >
                <strong>{item.name}</strong>
                {item.deleted_at && <span className="ml-2 text-muted-foreground">Deleted</span>}
                <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
                  <dt>Default version</dt>
                  <dd className="font-mono text-xs">{item.default_version_id ?? "None"}</dd>
                  <dt>Latest version</dt>
                  <dd>
                    {item.latest_version === null
                      ? "None"
                      : `${item.latest_version} · ${item.latest_version_id}`}
                  </dd>
                  <dt>Pinned Tasks</dt>
                  <dd>{item.pinned_task_count}</dd>
                  <dt>Catalog revision</dt>
                  <dd>{item.revision}</dd>
                </dl>
                {!item.deleted_at && (
                  <Button
                    variant="outline"
                    className="mt-3"
                    onClick={() => {
                      if ((managing || creating) && !scope.leaveGuard.canLeave()) return;
                      setCreating(false);
                      setManaging(managing === item.id ? null : item.id);
                    }}
                  >
                    Manage Pipeline
                  </Button>
                )}
                {managing === item.id && (
                  <PipelineCatalogActions
                    key={item.id}
                    {...scope}
                    item={item}
                    cursor={navigation.cursor}
                    onClose={() => setManaging(null)}
                    onApplied={() => {
                      setManaging(null);
                      void catalog.refetch();
                    }}
                  />
                )}
              </li>
            ))}
          </ul>
        )}
        <nav
          aria-label="Pipeline catalog pagination"
          className="flex flex-wrap items-center justify-between gap-3"
        >
          <Button
            variant="outline"
            disabled={catalog.isFetching || navigation.previous.length === 0}
            onClick={() =>
              navigate(navigation.previous.at(-1) ?? null, navigation.previous.slice(0, -1))
            }
          >
            Previous catalog page
          </Button>
          <span className="text-sm">Page {navigation.previous.length + 1}</span>
          <Button
            variant="outline"
            disabled={catalog.isFetching || catalog.isError || !catalog.data?.next_cursor}
            onClick={() => {
              const next = catalog.data?.next_cursor;
              if (next) navigate(next, [...navigation.previous, navigation.cursor]);
            }}
          >
            Next catalog page
          </Button>
        </nav>
      </section>
      <ProjectHooksBrowser {...scope} />
    </>
  );
}
