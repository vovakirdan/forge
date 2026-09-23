import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Field } from "./Field.tsx";
import { RunDetailPanel } from "./RunDetailPanel.tsx";
import { RunFacts } from "./RunFacts.tsx";
import { describeApiError, LiveApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { EmployeeEditPanel } from "./EmployeeEditPanel.tsx";
import { EmployeeLifecyclePanel } from "./EmployeeLifecyclePanel.tsx";
import { EmployeeInbox } from "./EmployeeInbox.tsx";
import { EmployeeOnboardingPanel } from "./EmployeeOnboardingPanel.tsx";
import { EmployeeRuntimePanel } from "./EmployeeRuntimePanel.tsx";

export function EmployeeProfilePanel({
  scope,
  employeeId,
  onClose,
}: {
  scope: ProjectReadScope;
  employeeId: string;
  onClose: () => void;
}) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const title = useRef<HTMLHeadingElement>(null);
  const runOpener = useRef<HTMLButtonElement | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [managing, setManaging] = useState(false);
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const profileKey = useMemo(
    () => readKeys.employee(generation, projectId, employeeId),
    [generation, projectId, employeeId],
  );
  const historyKey = useMemo(
    () => readKeys.employeeRuns(generation, projectId, employeeId, navigation.cursor),
    [generation, projectId, employeeId, navigation.cursor],
  );
  const operationsKey = useMemo(
    () => readKeys.employeeOperations(generation, projectId, employeeId),
    [generation, projectId, employeeId],
  );
  const profile = useQuery({
    queryKey: profileKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.employee(projectId, employeeId, token, signal)),
    retry: false,
  });
  const history = useQuery({
    queryKey: historyKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employeeRuns(projectId, employeeId, navigation.cursor, token, signal),
      ),
    enabled: profile.data !== undefined,
    retry: false,
  });
  const operations = useQuery({
    queryKey: operationsKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employeeOperations(projectId, employeeId, token, signal),
      ),
    enabled: profile.data !== undefined,
    retry: false,
  });
  useReadLifetime(profileKey);
  useReadLifetime(historyKey);
  useReadLifetime(operationsKey);
  useEffect(() => title.current?.focus(), []);

  function navigate(cursor: string | null, previous: (string | null)[]) {
    setRunId(null);
    prepareReadChange(queries, historyKey);
    setNavigation({ cursor, previous });
  }
  function closeRun() {
    setRunId(null);
    if (runOpener.current?.isConnected) runOpener.current.focus();
  }

  const employee = profile.data;
  const runs = history.data?.items;
  return (
    <>
      <section
        aria-label="Employee profile"
        className="space-y-5 rounded-xl border border-border bg-card p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2
            ref={title}
            tabIndex={-1}
            className="rounded text-lg font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            Employee profile
          </h2>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              disabled={
                !profile.data ||
                profile.isFetching ||
                profile.isError ||
                profile.data.state === "retired" ||
                editing ||
                managing
              }
              onClick={() => setEditing(true)}
            >
              Edit Employee
            </Button>
            <Button
              variant="outline"
              disabled={
                !profile.data || profile.isFetching || profile.isError || editing || managing
              }
              onClick={() => setManaging(true)}
            >
              Manage state
            </Button>
            <Button
              variant="outline"
              disabled={profile.isFetching}
              onClick={() => void profile.refetch()}
            >
              Refresh profile
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                if (scope.leaveGuard.canLeave()) onClose();
              }}
            >
              Close profile
            </Button>
          </div>
        </div>
        {profile.isPending && <p role="status">Loading Employee profile…</p>}
        {profile.isFetching && employee && (
          <p role="status">Refreshing profile… Previous data remains visible.</p>
        )}
        {profile.isError && (
          <p role="alert">
            {employee ? "Showing stale profile. " : ""}
            {describeApiError(profile.error, "Employee")}
          </p>
        )}
        {employee && (
          <>
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
              <Field label="Name">{employee.name}</Field>
              <Field label="ID">{employee.id}</Field>
              <Field label="Project ID">{employee.project_id}</Field>
              <Field label="Role">{employee.role}</Field>
              <Field label="Scheduling state">{employee.state}</Field>
              <Field label="Capacity">{employee.max_concurrent_runs} concurrent Runs</Field>
              <Field label="Revision">{employee.revision}</Field>
              <Field label="Created">{employee.created_at}</Field>
              <Field label="Updated">{employee.updated_at}</Field>
            </dl>
            <section aria-label="Stage eligibility" className="space-y-2">
              <h3 className="font-medium">Stage eligibility</h3>
              {employee.stage_eligibility.mode === "any" ? (
                <p>Any stage allowed by later scheduling policies.</p>
              ) : (
                <ul className="list-inside list-disc space-y-1 text-sm">
                  {employee.stage_eligibility.stages.map((stage) => (
                    <li
                      key={`${stage.pipeline_version_id}:${stage.stage_id}`}
                      className="[overflow-wrap:anywhere]"
                    >
                      {stage.stage_id} in Pipeline version {stage.pipeline_version_id}
                    </li>
                  ))}
                </ul>
              )}
            </section>
            <section
              aria-label="Employee operations"
              className="space-y-3 border-t border-border pt-4"
            >
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="font-medium">Operations</h3>
                <Button
                  variant="outline"
                  disabled={operations.isFetching}
                  onClick={() => void operations.refetch()}
                >
                  Refresh operations
                </Button>
              </div>
              {operations.isPending && <p role="status">Loading operational evidence…</p>}
              {operations.isError && (
                <p role="alert">
                  {operations.data ? "Showing stale operational evidence. " : ""}
                  {describeApiError(operations.error, "Employee")}
                </p>
              )}
              {operations.data && (
                <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
                  <Field label="Occupied slots">
                    {operations.data.occupied_slots} / {operations.data.max_concurrent_runs}
                  </Field>
                  <Field label="Observed running Runs">
                    {operations.data.observed_running_runs}
                  </Field>
                  <Field label="Runtime binding">
                    {operations.data.runtime_binding_configured ? "Configured" : "Unavailable"}
                  </Field>
                  <Field label="Availability">Unknown</Field>
                </dl>
              )}
              <p className="text-xs text-muted-foreground">
                Occupied slots and observed Runs are separate facts. Availability cannot be
                confirmed from these records.
              </p>
            </section>
            <EmployeeRuntimePanel scope={scope} employeeId={employeeId} />
            <EmployeeOnboardingPanel
              key={`onboarding:${employeeId}`}
              scope={scope}
              employeeId={employeeId}
            />
            <EmployeeInbox key={`inbox:${employeeId}`} scope={scope} employeeId={employeeId} />
            <section
              aria-label="Employee Run history"
              className="space-y-4 border-t border-border pt-4"
            >
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="font-medium">Run history</h3>
                <Button
                  variant="outline"
                  disabled={history.isFetching}
                  onClick={() => void history.refetch()}
                >
                  Refresh history
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                Retained Runs assigned to this Employee, newest first. Desired and observed states
                are separate facts; this list does not show live availability.
              </p>
              {history.isPending && <p role="status">Loading Run history…</p>}
              {history.isFetching && runs && (
                <p role="status">Refreshing Run history… Previous data remains visible.</p>
              )}
              {history.isError && (
                <p role="alert">
                  {runs ? "Showing stale Run history. " : ""}
                  {describeApiError(history.error, "Run")}
                </p>
              )}
              {history.error instanceof LiveApiError && history.error.kind === "cursor_invalid" && (
                <Button
                  variant="secondary"
                  onClick={() => {
                    if (navigation.cursor === null) void history.refetch();
                    else navigate(null, []);
                  }}
                >
                  Restart Run history
                </Button>
              )}
              {runs?.length === 0 && (
                <p>
                  {navigation.cursor === null
                    ? "No Runs for this Employee."
                    : "No Runs on this page."}
                </p>
              )}
              {runs && runs.length > 0 && (
                <ul aria-label="Employee Runs" className="space-y-3">
                  {runs.map((run) => (
                    <li key={run.id} className="space-y-2 rounded-lg border border-border p-4">
                      <button
                        type="button"
                        aria-label={`Open Run ${run.id}`}
                        onClick={(event) => {
                          runOpener.current = event.currentTarget;
                          setRunId(run.id);
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
                aria-label="Employee Run pagination"
                className="flex flex-wrap items-center justify-between gap-3"
              >
                <Button
                  variant="outline"
                  disabled={history.isFetching || navigation.previous.length === 0}
                  onClick={() =>
                    navigate(navigation.previous.at(-1) ?? null, navigation.previous.slice(0, -1))
                  }
                >
                  Previous Run page
                </Button>
                <p className="text-sm">Page {navigation.previous.length + 1}</p>
                <Button
                  variant="outline"
                  disabled={
                    history.isFetching || history.isError || history.data?.next_cursor === undefined
                  }
                  onClick={() => {
                    const next = history.data?.next_cursor;
                    if (next !== undefined)
                      navigate(next, [...navigation.previous, navigation.cursor]);
                  }}
                >
                  Next Run page
                </Button>
              </nav>
            </section>
          </>
        )}
      </section>
      {editing && employee && (
        <EmployeeEditPanel
          scope={scope}
          employee={employee}
          onCancel={() => setEditing(false)}
          onSaved={() => setEditing(false)}
        />
      )}
      {managing && employee && (
        <EmployeeLifecyclePanel
          {...scope}
          employeeId={employee.id}
          onClose={() => setManaging(false)}
        />
      )}
      {runId && <RunDetailPanel key={runId} {...scope} runId={runId} onClose={closeRun} />}
    </>
  );
}
