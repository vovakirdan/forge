import { useEffect, useMemo, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { RunFacts, RunOwner } from "./RunFacts.tsx";
import { RunDiagnostics } from "./RunDiagnostics.tsx";
import { RunEvidencePanel } from "./RunEvidencePanel.tsx";

export function RunDetailPanel({
  api,
  session,
  generation,
  projectId,
  runId,
  leaveGuard,
  onClose,
}: ProjectReadScope & { runId: string; onClose: () => void }) {
  const title = useRef<HTMLHeadingElement>(null);
  const key = useMemo(
    () => readKeys.run(generation, projectId, runId),
    [generation, projectId, runId],
  );
  const detail = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.run(projectId, runId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  useEffect(() => {
    title.current?.focus();
  }, []);
  const run = detail.data;
  return (
    <section
      id="run-details"
      aria-label="Run details"
      className="space-y-5 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2
          id="run-details-title"
          ref={title}
          tabIndex={-1}
          className="rounded text-lg font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Run details
        </h2>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            disabled={detail.isFetching}
            onClick={() => void detail.refetch()}
          >
            Refresh run
          </Button>
          <Button variant="outline" onClick={onClose}>
            Close run
          </Button>
        </div>
      </div>
      {detail.isPending && <p role="status">Loading Run details…</p>}
      {detail.isFetching && run && (
        <p role="status">Refreshing Run details… Previous data remains visible.</p>
      )}
      {detail.isError && (
        <p role="alert">
          {run ? "Showing stale Run details. " : ""}
          {describeApiError(detail.error, "Run")}
        </p>
      )}
      {run && (
        <>
          <RunFacts run={run} detail />
          <RunOwner run={run} />
          <RunDiagnostics diagnostics={run.diagnostics} />
          <RunEvidencePanel {...{ api, session, generation, projectId, runId, leaveGuard }} />
        </>
      )}
    </section>
  );
}
