import { useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError, LiveApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { EmployeeProfilePanel } from "./EmployeeProfilePanel.tsx";
import { EmployeeCreatePanel } from "./EmployeeCreatePanel.tsx";

export function TeamBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [employeeId, setEmployeeId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const opener = useRef<HTMLButtonElement | null>(null);
  const createOpener = useRef<HTMLButtonElement | null>(null);
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const key = useMemo(
    () => readKeys.employees(generation, projectId, navigation.cursor),
    [generation, projectId, navigation.cursor],
  );
  const employees = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employees(projectId, navigation.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  function navigate(cursor: string | null, previous: (string | null)[]) {
    setEmployeeId(null);
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  function closeProfile() {
    setEmployeeId(null);
    if (opener.current?.isConnected) opener.current.focus();
  }
  function created(id: string) {
    setCreating(false);
    setNavigation({ cursor: null, previous: [] });
    setEmployeeId(id);
  }
  const items = employees.data?.items;
  return (
    <>
      <section aria-label="Team" className="space-y-4 rounded-xl border border-border bg-card p-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Team</h2>
          <div className="flex flex-wrap gap-2">
            <Button
              ref={createOpener}
              disabled={creating}
              onClick={() => {
                setEmployeeId(null);
                setCreating(true);
              }}
            >
              Create Employee
            </Button>
            <Button
              variant="outline"
              disabled={employees.isFetching}
              onClick={() => void employees.refetch()}
            >
              Refresh team
            </Button>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          Project roster, up to 20 Employees per page. State controls eligibility for new work; it
          does not show live availability.
        </p>
        {employees.isPending && <p role="status">Loading team…</p>}
        {employees.isFetching && items && (
          <p role="status">Refreshing team… Previous data remains visible.</p>
        )}
        {employees.isError && (
          <p role="alert">
            {items ? "Showing stale team. " : ""}
            {describeApiError(employees.error, "Employee")}
          </p>
        )}
        {employees.error instanceof LiveApiError && employees.error.kind === "cursor_invalid" && (
          <Button
            variant="secondary"
            onClick={() => {
              if (navigation.cursor === null) void employees.refetch();
              else navigate(null, []);
            }}
          >
            Restart team pages
          </Button>
        )}
        {items?.length === 0 && (
          <p>
            {navigation.cursor === null
              ? "No Employees in this project."
              : "No Employees on this page."}
          </p>
        )}
        {items && items.length > 0 && (
          <ul className="space-y-3" aria-label="Employee list">
            {items.map((employee) => (
              <li
                key={employee.id}
                className="rounded-lg border border-border p-4 [overflow-wrap:anywhere]"
              >
                <button
                  type="button"
                  aria-label={`Open Employee ${employee.name}`}
                  onClick={(event) => {
                    opener.current = event.currentTarget;
                    setEmployeeId(employee.id);
                  }}
                  className="cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {employee.name}
                </button>
                <dl className="mt-2 grid gap-x-4 gap-y-1 text-sm sm:grid-cols-2">
                  <dt className="text-muted-foreground">Role</dt>
                  <dd>{employee.role}</dd>
                  <dt className="text-muted-foreground">State</dt>
                  <dd>{employee.state}</dd>
                  <dt className="text-muted-foreground">Capacity</dt>
                  <dd>{employee.max_concurrent_runs} concurrent Runs</dd>
                  <dt className="text-muted-foreground">Revision</dt>
                  <dd>{employee.revision}</dd>
                  <dt className="text-muted-foreground">ID</dt>
                  <dd>{employee.id}</dd>
                </dl>
              </li>
            ))}
          </ul>
        )}
        <nav
          aria-label="Team pagination"
          className="flex flex-wrap items-center justify-between gap-3"
        >
          <Button
            variant="outline"
            disabled={employees.isFetching || navigation.previous.length === 0}
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
              employees.isFetching || employees.isError || employees.data?.next_cursor === undefined
            }
            onClick={() => {
              const next = employees.data?.next_cursor;
              if (next !== undefined) navigate(next, [...navigation.previous, navigation.cursor]);
            }}
          >
            Next page
          </Button>
        </nav>
      </section>
      {creating && (
        <EmployeeCreatePanel
          {...scope}
          onCreated={created}
          onCancel={() => {
            setCreating(false);
            if (createOpener.current?.isConnected) createOpener.current.focus();
          }}
        />
      )}
      {employeeId && (
        <EmployeeProfilePanel
          key={employeeId}
          scope={scope}
          employeeId={employeeId}
          onClose={closeProfile}
        />
      )}
    </>
  );
}
