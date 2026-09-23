import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import {
  EmployeeLifecycleRequestSchema,
  employeeLifecycleAttempt,
  type EmployeeLifecycleAction,
  type EmployeeLifecycleAttempt,
} from "../contracts/employee-lifecycle.ts";
import type { EmployeeCommandReceipt } from "../contracts/create-employee.ts";
import type { EmployeeProfile } from "../contracts/employee.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type State =
  | { status: "idle" }
  | { status: "sending" | "unknown"; attempt: EmployeeLifecycleAttempt }
  | { status: "refused"; error: LiveCommandError }
  | {
      status: "accepted";
      receipt: EmployeeCommandReceipt;
      action: EmployeeLifecycleAction;
      readError: boolean;
    };

const labels = {
  enable_employee: "Enable Employee",
  disable_employee: "Disable Employee",
  retire_employee: "Retire Employee",
} as const;

const expectedState = {
  enable_employee: "enabled",
  disable_employee: "disabled",
  retire_employee: "retired",
} as const;

function availableActions(state: EmployeeProfile["state"]): EmployeeLifecycleAction[] {
  if (state === "enabled") return ["disable_employee", "retire_employee"];
  if (state === "disabled") return ["enable_employee", "retire_employee"];
  return [];
}

function refusal(error: LiveCommandError) {
  switch (error.kind) {
    case "stale_revision":
      return "Project or Employee changed. Refresh both before choosing an action again.";
    case "idempotency_conflict":
      return "This request key belongs to another command. Refresh before trying again.";
    case "validation_failed":
    case "invalid_request":
      return "Core refused this change. Check the reason and refresh before trying again.";
    case "not_found":
    case "forbidden":
      return "Employee change was refused. Refresh before trying again.";
    case "outcome_unknown":
      return "Outcome unknown. Retry the same request to obtain its receipt.";
  }
}

type Props = ProjectReadScope & { employeeId: string; onClose: () => void };

export function EmployeeLifecyclePanel(scope: Props) {
  const { api, session, generation, projectId, employeeId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [action, setAction] = useState<EmployeeLifecycleAction | null>(null);
  const [reason, setReason] = useState("");
  const [state, setState] = useState<State>({ status: "idle" });
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const title = useRef<HTMLHeadingElement>(null);
  const confirmationTitle = useRef<HTMLHeadingElement>(null);
  const baselineKey = useMemo(
    () => ["live", generation, projectId, "employee-lifecycle-baseline", employeeId] as const,
    [generation, projectId, employeeId],
  );
  const projectKey = useMemo(
    () => ["project", generation, projectId] as const,
    [generation, projectId],
  );
  const profileKey = useMemo(
    () => readKeys.employee(generation, projectId, employeeId),
    [generation, projectId, employeeId],
  );
  const baseline = useQuery({
    queryKey: baselineKey,
    queryFn: async ({ signal }) => {
      const [project, employee] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, signal)),
        session.request(generation, (token) => api.employee(projectId, employeeId, token, signal)),
      ]);
      return { project, employee };
    },
    retry: false,
  });
  useReadLifetime(baselineKey);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    title.current?.focus();
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (action) confirmationTitle.current?.focus();
  }, [action]);
  const guard = state.status === "sending" || state.status === "unknown";
  useEffect(() => {
    if (!guard) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Employee state change? An in-flight or unknown command may still have been applied by Core; leaving does not undo it.",
    );
    function beforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "";
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [leaveGuard, guard]);

  function current(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }

  async function readAccepted(
    receipt: EmployeeCommandReceipt,
    acceptedAction: EmployeeLifecycleAction,
    controller: AbortController,
  ) {
    setState({ status: "accepted", receipt, action: acceptedAction, readError: false });
    try {
      const [project, employee] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
        session.request(generation, (token) =>
          api.employee(projectId, employeeId, token, controller.signal),
        ),
      ]);
      if (!current(controller)) return;
      if (
        project.revision < receipt.project_revision ||
        employee.state !== expectedState[acceptedAction] ||
        employee.revision <= (baseline.data?.employee.revision ?? 0)
      )
        throw new Error("Employee readback did not confirm the command");
      await Promise.all([
        queries.cancelQueries({ queryKey: projectKey, exact: true }),
        queries.cancelQueries({ queryKey: profileKey, exact: true }),
      ]);
      if (!current(controller)) return;
      queries.setQueryData(projectKey, project);
      queries.setQueryData(profileKey, employee);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "employees"] },
        { throwOnError: true },
      );
      if (current(controller)) scope.onClose();
    } catch {
      if (current(controller))
        setState({ status: "accepted", receipt, action: acceptedAction, readError: true });
    }
  }

  async function submit(attempt: EmployeeLifecycleAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.employeeLifecycle(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (current(controller)) await readAccepted(receipt, attempt.action, controller);
    } catch (error) {
      if (current(controller))
        setState(
          error instanceof LiveCommandError && error.kind !== "outcome_unknown"
            ? { status: "refused", error }
            : { status: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }

  async function refreshBaseline() {
    const controller = lifetime.current;
    if (!controller || baseline.isFetching) return;
    setRefreshError(null);
    const fresh = await baseline.refetch();
    if (!current(controller)) return;
    if (fresh.isError || !fresh.data) {
      setRefreshError(describeApiError(fresh.error, "Employee"));
      return;
    }
    await Promise.all([
      queries.cancelQueries({ queryKey: projectKey, exact: true }),
      queries.cancelQueries({ queryKey: profileKey, exact: true }),
    ]);
    if (!current(controller)) return;
    queries.setQueryData(projectKey, fresh.data.project);
    queries.setQueryData(profileKey, fresh.data.employee);
    void queries.invalidateQueries({ queryKey: ["live", generation, projectId, "employees"] });
    setAction(null);
    setState({ status: "idle" });
  }

  const employee = baseline.data?.employee;
  const validReason =
    reason.trim() === "" ||
    EmployeeLifecycleRequestSchema.shape.payload.shape.reason.safeParse(reason).success;
  const canConfirm = Boolean(
    action &&
    employee &&
    availableActions(employee.state).includes(action) &&
    baseline.data &&
    !baseline.isFetching &&
    !baseline.isError &&
    validReason &&
    baseline.data.project.revision < Number.MAX_SAFE_INTEGER &&
    employee.revision < Number.MAX_SAFE_INTEGER,
  );

  function confirm() {
    if (!canConfirm || !action || !baseline.data) return;
    const input = EmployeeLifecycleRequestSchema.parse({
      project_id: projectId,
      expected_revision: baseline.data.project.revision,
      payload: {
        employee_id: employeeId,
        expected_employee_revision: baseline.data.employee.revision,
        ...(reason.trim() ? { reason: reason.trim() } : {}),
      },
    });
    setAction(null);
    void submit(employeeLifecycleAttempt(action, input));
  }

  return (
    <section
      aria-label="Manage Employee state"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <h3
        ref={title}
        tabIndex={-1}
        className="rounded font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        Manage Employee state
      </h3>
      <p className="text-sm text-muted-foreground">
        These actions control future assignments. They do not stop active Runs or show live
        availability.
      </p>
      {baseline.isPending && <p role="status">Loading current Project and Employee…</p>}
      {baseline.isError && <p role="alert">{describeApiError(baseline.error, "Employee")}</p>}
      {employee && (
        <p className="text-sm">
          {employee.name} is {employee.state}.
        </p>
      )}
      {employee?.state === "retired" && <p>Retirement is final. Run history remains available.</p>}
      {employee && state.status === "idle" && (
        <div className="flex flex-wrap gap-2">
          {availableActions(employee.state).map((candidate) => (
            <Button
              key={candidate}
              variant={candidate === "retire_employee" ? "destructive" : "outline"}
              disabled={baseline.isFetching || baseline.isError}
              onClick={() => {
                setReason("");
                setAction(candidate);
              }}
            >
              {labels[candidate]}
            </Button>
          ))}
        </div>
      )}
      {state.status === "sending" && <p role="status">Sending Employee state change…</p>}
      {state.status === "unknown" && (
        <div className="space-y-2">
          <p role="alert">Outcome unknown. Retry the same request to obtain its receipt.</p>
          <Button disabled={inFlight.current} onClick={() => void submit(state.attempt)}>
            Retry same request
          </Button>
        </div>
      )}
      {state.status === "refused" && (
        <div className="space-y-2">
          <p role="alert">{refusal(state.error)}</p>
          <Button disabled={baseline.isFetching} onClick={() => void refreshBaseline()}>
            Refresh Project and Employee
          </Button>
        </div>
      )}
      {state.status === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Change accepted. Receipt {state.receipt.command_id}.{" "}
            {state.readError ? "Profile readback failed." : "Reading canonical profile…"}
          </p>
          {state.readError && (
            <Button
              onClick={() => {
                if (lifetime.current)
                  void readAccepted(state.receipt, state.action, lifetime.current);
              }}
            >
              Retry profile read
            </Button>
          )}
        </div>
      )}
      {refreshError && <p role="alert">{refreshError}</p>}
      <Button
        variant="outline"
        onClick={() => {
          if (leaveGuard.canLeave()) scope.onClose();
        }}
      >
        Close state controls
      </Button>

      {action && (
        <div
          role="alertdialog"
          aria-labelledby="employee-state-confirm-title"
          aria-describedby="employee-state-confirm-description"
          className="space-y-3 rounded-lg border border-border p-4"
        >
          <h4
            id="employee-state-confirm-title"
            ref={confirmationTitle}
            tabIndex={-1}
            className="rounded font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {labels[action]}: {employee?.name ?? employeeId}
          </h4>
          <p id="employee-state-confirm-description" className="text-sm">
            {action === "retire_employee"
              ? "Retirement is final. This Employee cannot be enabled again. Existing Run history remains; active Runs are not stopped."
              : action === "disable_employee"
                ? "New assignments will be blocked. Active Runs are not stopped."
                : "New assignments may be admitted again, subject to other Core gates."}
          </p>
          <label className="block space-y-1 text-sm">
            <span>Reason (optional)</span>
            <textarea
              className="block w-full rounded border border-border bg-background p-2"
              value={reason}
              onChange={(event) => setReason(event.target.value)}
            />
          </label>
          {!validReason && (
            <p role="alert">Reason must be at most 10,000 characters and contain no NUL.</p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => setAction(null)}>
              Keep current state
            </Button>
            <Button disabled={!canConfirm} onClick={confirm}>
              Confirm {labels[action]}
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}
