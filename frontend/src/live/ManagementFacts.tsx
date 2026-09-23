import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { RecoveryAssessmentAction } from "./RecoveryAssessmentAction.tsx";

type Kind = "resolver-routes" | "next-run-constraints" | "recovery-runs";
type Page = { cursor: string | null; previous: (string | null)[] };
const FIRST: Page = { cursor: null, previous: [] };

export function ManagementFacts({ scope }: { scope: ProjectReadScope }) {
  const { api, session, generation, projectId } = scope;
  const key = useMemo(
    () => readKeys.recoverySettings(generation, projectId),
    [generation, projectId],
  );
  const settings = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.recoverySettings(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section aria-label="Resolver and recovery facts" className="space-y-4">
      <section
        className="space-y-2 rounded-xl border border-border p-4"
        aria-label="Recovery settings"
      >
        <div className="flex justify-between">
          <h3 className="font-medium">Recovery settings</h3>
          <Button
            variant="outline"
            onClick={() => void settings.refetch()}
            disabled={settings.isFetching}
          >
            Refresh
          </Button>
        </div>
        {settings.isPending && <p role="status">Loading recovery settings…</p>}
        {settings.isError && <p role="alert">{describeApiError(settings.error)}</p>}
        {settings.data && (
          <p className="text-sm">
            Boot policy: {settings.data.boot_policy}. Recovery hold:{" "}
            {settings.data.hold ? "on" : "off"}. This is policy, not proof that any Run is safe to
            retry.
          </p>
        )}
      </section>
      <FactPage scope={scope} kind="resolver-routes" title="Resolver routes" />
      <FactPage scope={scope} kind="next-run-constraints" title="Next Run constraints" />
      <FactPage scope={scope} kind="recovery-runs" title="Run recovery observations" />
    </section>
  );
}

function FactPage({ scope, kind, title }: { scope: ProjectReadScope; kind: Kind; title: string }) {
  const { api, session, generation, projectId } = scope;
  const client = useQueryClient();
  const [page, setPage] = useState<Page>(FIRST);
  const key = useMemo(
    () => readKeys.managementFactsPage(generation, projectId, kind, page.cursor),
    [generation, projectId, kind, page.cursor],
  );
  const query = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.managementFactsPage(projectId, kind, page.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  function navigate(next: Page) {
    prepareReadChange(client, key);
    setPage(next);
  }
  const value = query.data?.value;
  return (
    <section aria-label={title} className="space-y-3 rounded-xl border border-border p-4">
      <div className="flex justify-between gap-2">
        <h3 className="font-medium">{title}</h3>
        <Button variant="outline" onClick={() => void query.refetch()} disabled={query.isFetching}>
          Refresh
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        20 records per page. This page is a bounded slice of Project history.
      </p>
      {query.isPending && <p role="status">Loading {title.toLowerCase()}…</p>}
      {query.isError && (
        <p role="alert">
          {value ? "Showing stale records. " : ""}
          {describeApiError(query.error)}
        </p>
      )}
      {value?.items.length === 0 && <p>No records on this page.</p>}
      {query.data?.kind === "resolver-routes" && (
        <ul className="space-y-2 text-sm">
          {query.data.value.items.map((route) => (
            <li key={route.key} className="rounded border p-3 [overflow-wrap:anywhere]">
              <strong>{route.key}</strong> · revision {route.revision} · timeout{" "}
              {route.assignment_timeout_seconds}s<br />
              Employees: {route.employee_ids.join(", ") || "none"}
              <br />
              Updated {route.updated_at}
            </li>
          ))}
        </ul>
      )}
      {query.data?.kind === "next-run-constraints" && (
        <ul className="space-y-2 text-sm">
          {query.data.value.items.map((constraint) => (
            <li key={constraint.id} className="rounded border p-3 [overflow-wrap:anywhere]">
              <strong>{constraint.state.status}</strong> · Task {constraint.task_id} · Employee{" "}
              {constraint.employee_id}
              <br />
              Stage {constraint.stage_id}, visit {constraint.stage_visit} · constraint{" "}
              {constraint.id}
              <br />
              Created {constraint.created_at}
            </li>
          ))}
        </ul>
      )}
      {query.data?.kind === "recovery-runs" && (
        <ul className="space-y-2 text-sm">
          {query.data.value.items.map((run) => (
            <li key={run.run_id} className="rounded border p-3 [overflow-wrap:anywhere]">
              <strong>Run {run.run_id}</strong> ·{" "}
              {run.task_id ? `Task ${run.task_id}` : "Taskless Run"}
              <br />
              Desired {run.desired_state}; observed {run.observed_state}; lease{" "}
              {run.lease_active ? "active" : "retired"}
              <br />
              Reservation {run.reservation_state ?? "absent"}; released{" "}
              {run.reservation_released === null
                ? "unknown"
                : run.reservation_released
                  ? "yes"
                  : "no"}
              <br />
              Accepted assessment {run.accepted_assessment ?? "none"}; accepted at{" "}
              {run.assessment_accepted_at ?? "not recorded"}
              <br />
              Last updated {run.updated_at}
              <RecoveryAssessmentAction scope={scope} runId={run.run_id} />
            </li>
          ))}
        </ul>
      )}
      {kind === "recovery-runs" && (
        <p className="text-xs text-muted-foreground">
          These observations do not include all evidence needed to accept a safe re-execution
          assessment. No retryability is inferred here.
        </p>
      )}
      <nav aria-label={`${title} pagination`} className="flex justify-between gap-2">
        <Button
          variant="outline"
          disabled={query.isFetching || page.previous.length === 0}
          onClick={() =>
            navigate({ cursor: page.previous.at(-1) ?? null, previous: page.previous.slice(0, -1) })
          }
        >
          Previous
        </Button>
        <span>Page {page.previous.length + 1}</span>
        <Button
          variant="outline"
          disabled={query.isFetching || !value?.next_cursor}
          onClick={() => {
            if (value?.next_cursor)
              navigate({ cursor: value.next_cursor, previous: [...page.previous, page.cursor] });
          }}
        >
          Next
        </Button>
      </nav>
    </section>
  );
}
