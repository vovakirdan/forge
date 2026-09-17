import { useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError, LiveApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { RunFacts } from "./RunFacts.tsx";
import { RunDetailPanel } from "./RunDetailPanel.tsx";

export function RunBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const [runId, setRunId] = useState<string | null>(null);
  const opener = useRef<HTMLButtonElement | null>(null);
  const key = useMemo(
    () => readKeys.runs(generation, projectId, navigation.cursor),
    [generation, projectId, navigation.cursor],
  );
  const runs = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.runs(projectId, navigation.cursor, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  function closeRun() {
    setRunId(null);
    if (opener.current?.isConnected) opener.current.focus();
  }
  function navigate(cursor: string | null, previous: (string | null)[]) {
    setRunId(null);
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  const items = runs.data?.items;
  return (
    <>
      <section aria-label="Runs" className="space-y-4 rounded-xl border border-border bg-card p-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Runs</h2>
          <Button variant="outline" disabled={runs.isFetching} onClick={() => void runs.refetch()}>
            Refresh runs
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          Read-only Project-wide list, up to 20 Runs per page. Desired and observed states are
          separate facts; neither is Task lifecycle.
        </p>
        {runs.isPending && <p role="status">Loading runs…</p>}
        {runs.isFetching && items && (
          <p role="status">Refreshing runs… Previous data remains visible.</p>
        )}
        {runs.isError && (
          <p role="alert">
            {items ? "Showing stale runs. " : ""}
            {describeApiError(runs.error, "Run")}
          </p>
        )}
        {runs.error instanceof LiveApiError && runs.error.kind === "cursor_invalid" && (
          <Button
            variant="secondary"
            onClick={() => {
              if (navigation.cursor === null) void runs.refetch();
              else navigate(null, []);
            }}
          >
            Restart pagination
          </Button>
        )}
        {items?.length === 0 && (
          <p>{navigation.cursor === null ? "No runs in this project." : "No runs on this page."}</p>
        )}
        {items && items.length > 0 && (
          <ul className="space-y-3" aria-label="Run list">
            {items.map((run) => (
              <li key={run.id} className="space-y-3 rounded-lg border border-border p-4">
                <button
                  type="button"
                  aria-label={`Open Run ${run.id}`}
                  aria-controls="run-details"
                  onClick={(event) => {
                    opener.current = event.currentTarget;
                    if (runId === run.id) document.getElementById("run-details-title")?.focus();
                    else setRunId(run.id);
                  }}
                  className="w-full cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
                >
                  Run {run.id}
                </button>
                <RunFacts run={run} />
              </li>
            ))}
          </ul>
        )}
        <nav
          aria-label="Run pagination"
          className="flex flex-wrap items-center justify-between gap-3"
        >
          <Button
            variant="outline"
            disabled={runs.isFetching || navigation.previous.length === 0}
            onClick={() => {
              navigate(navigation.previous.at(-1) ?? null, navigation.previous.slice(0, -1));
            }}
          >
            Previous page
          </Button>
          <p className="text-sm">Page {navigation.previous.length + 1}</p>
          <Button
            variant="outline"
            disabled={runs.isFetching || runs.isError || runs.data?.next_cursor === undefined}
            onClick={() => {
              const next = runs.data?.next_cursor;
              if (next !== undefined) navigate(next, [...navigation.previous, navigation.cursor]);
            }}
          >
            Next page
          </Button>
        </nav>
      </section>
      {runId && <RunDetailPanel key={runId} {...scope} runId={runId} onClose={closeRun} />}
    </>
  );
}
