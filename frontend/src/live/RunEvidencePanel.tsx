import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { Field } from "./Field.tsx";

export function RunEvidencePanel({
  api,
  session,
  generation,
  projectId,
  runId,
}: ProjectReadScope & { runId: string }) {
  const [cursor, setCursor] = useState<string | null>(null);
  const contextKey = useMemo(
    () => readKeys.runContext(generation, projectId, runId),
    [generation, projectId, runId],
  );
  const evidenceKey = useMemo(
    () => readKeys.runEvidence(generation, projectId, runId, cursor),
    [generation, projectId, runId, cursor],
  );
  const context = useQuery({
    queryKey: contextKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.runContext(projectId, runId, token, signal)),
    retry: false,
  });
  const evidence = useQuery({
    queryKey: evidenceKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.runEvidence(projectId, runId, cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(contextKey);
  useReadLifetime(evidenceKey);

  return (
    <section aria-label="Run context and evidence" className="space-y-4">
      <h3 className="font-medium">Run context and evidence</h3>
      <p className="text-xs text-muted-foreground">
        Coordinates and receipts identify technical evidence. They do not prove Task acceptance.
        Evidence content and dispatch context bodies are unavailable in this view.
      </p>
      <section aria-label="Run context coordinates" className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <h4 className="text-sm font-medium">Dispatch context</h4>
          <Button
            variant="outline"
            disabled={context.isFetching}
            onClick={() => void context.refetch()}
          >
            Refresh context
          </Button>
        </div>
        {context.isPending && <p role="status">Loading Run context…</p>}
        {context.isError && <p role="alert">{describeApiError(context.error, "Run")}</p>}
        {context.data?.availability === "unavailable" && <p>Context snapshot unavailable.</p>}
        {context.data?.availability === "available" && (
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 break-all text-sm">
            <Field label="Snapshot">{context.data.coordinates.context_snapshot_id}</Field>
            <Field label="Task">{context.data.coordinates.task_id}</Field>
            <Field label="Employee">{context.data.coordinates.employee_id}</Field>
            <Field label="Pipeline version">{context.data.coordinates.pipeline_version_id}</Field>
            <Field label="Stage">{context.data.coordinates.stage_id}</Field>
            <Field label="Stage visit">
              {context.data.coordinates.stage_visit ?? "Unavailable"}
            </Field>
            <Field label="Task revision before dispatch">
              {context.data.coordinates.task_revision_before_dispatch}
            </Field>
            <Field label="Run specification">{context.data.coordinates.run_spec_id}</Field>
          </dl>
        )}
      </section>
      <section aria-label="Run evidence receipts" className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <h4 className="text-sm font-medium">Evidence receipts</h4>
          <Button
            variant="outline"
            disabled={evidence.isFetching}
            onClick={() => void evidence.refetch()}
          >
            Refresh evidence
          </Button>
        </div>
        {evidence.isPending && <p role="status">Loading evidence receipts…</p>}
        {evidence.isError && <p role="alert">{describeApiError(evidence.error, "Run")}</p>}
        {evidence.data?.items.length === 0 && <p>No evidence receipts on this page.</p>}
        {evidence.data && evidence.data.items.length > 0 && (
          <ul className="space-y-2">
            {evidence.data.items.map((item) => (
              <li key={item.id} className="rounded border border-border p-3 text-sm">
                <div className="font-medium">
                  {item.stream} #{item.sequence}
                </div>
                <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 break-all">
                  <Field label="Receipt">{item.id}</Field>
                  <Field label="Storage">{item.storage_state}</Field>
                  <Field label="Size">{item.size_bytes} bytes</Field>
                  <Field label="SHA-256">{item.sha256}</Field>
                  <Field label="Redaction policy">{item.redaction_policy_reference}</Field>
                  <Field label="Created">{item.created_at}</Field>
                  <Field label="Content">Unavailable</Field>
                </dl>
              </li>
            ))}
          </ul>
        )}
        <div className="flex gap-2">
          {cursor && (
            <Button variant="outline" onClick={() => setCursor(null)}>
              First page
            </Button>
          )}
          {evidence.data?.next_cursor && (
            <Button variant="outline" onClick={() => setCursor(evidence.data.next_cursor ?? null)}>
              Next page
            </Button>
          )}
        </div>
      </section>
    </section>
  );
}
