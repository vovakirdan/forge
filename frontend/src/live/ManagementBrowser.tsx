import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { UuidV7Schema } from "../contracts/common.ts";
import {
  managementAttempt,
  type ManagementAction,
  type ManagementAttempt,
  type ManagementReceipt,
} from "../contracts/management-command.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { EscalationActions } from "./EscalationActions.tsx";
import { ManagementFacts } from "./ManagementFacts.tsx";
import { ManagerPlanningActions } from "./ManagerPlanningActions.tsx";
import { ResolverActions } from "./ResolverActions.tsx";

type Page = { cursor: string | null; previous: (string | null)[] };
const FIRST: Page = { cursor: null, previous: [] };

export function ManagementBrowser(scope: ProjectReadScope) {
  return (
    <section aria-label="Management and recovery" className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Management and recovery</h2>
        <p className="text-sm text-muted-foreground">
          Commands record intent. A stop request never proves a Run has physically stopped. Core
          checks every current revision and execution fence.
        </p>
      </div>
      <ManagementControls scope={scope} />
      <ManagerPlanningActions scope={scope} />
      <ResolverActions scope={scope} />
      <ManagementList scope={scope} kind="resume-schedules" title="Scheduled resumes" />
      <ManagementList scope={scope} kind="escalations" title="Escalations" />
      <ManagementFacts scope={scope} />
    </section>
  );
}

function ManagementList({
  scope,
  kind,
  title,
}: {
  scope: ProjectReadScope;
  kind: "resume-schedules" | "escalations";
  title: string;
}) {
  const { api, session, generation, projectId } = scope;
  const client = useQueryClient();
  const [page, setPage] = useState<Page>(FIRST);
  const key = useMemo(
    () => readKeys.managementPage(generation, projectId, kind, page.cursor),
    [generation, projectId, kind, page.cursor],
  );
  const query = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.managementPage(projectId, kind, page.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  const result = query.data;
  const value = result?.value;
  function navigate(next: Page) {
    prepareReadChange(client, key);
    setPage(next);
  }
  return (
    <section aria-label={title} className="space-y-3 rounded-xl border border-border p-4">
      <div className="flex justify-between gap-2">
        <h3 className="font-medium">{title}</h3>
        <Button variant="outline" disabled={query.isFetching} onClick={() => void query.refetch()}>
          Refresh
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        20 records per page. This page does not represent the full Project history.
      </p>
      {query.isPending && <p role="status">Loading {title.toLowerCase()}…</p>}
      {query.isError && (
        <p role="alert">
          {value ? "Showing stale records. " : ""}
          {describeApiError(query.error)}
        </p>
      )}
      {value?.items.length === 0 && <p>No records on this page.</p>}
      {result?.kind === "resume-schedules" && (
        <ul className="space-y-2">
          {result.value.items.map((schedule) => (
            <li key={schedule.id} className="rounded border p-3 text-sm [overflow-wrap:anywhere]">
              <strong>Task {schedule.task_id}</strong> · {schedule.state.status} · due{" "}
              {schedule.not_before}
              <br />
              Schedule {schedule.id} · wait {schedule.wait_condition_id} · stage {schedule.stage_id}
              , visit {schedule.stage_visit}
              <br />
              Reason: {schedule.reason}
            </li>
          ))}
        </ul>
      )}
      {result?.kind === "escalations" && (
        <ul className="space-y-2">
          {result.value.items.map((escalation) => (
            <li key={escalation.id} className="rounded border p-3 text-sm [overflow-wrap:anywhere]">
              <strong>{escalation.category}</strong> · {escalation.state.status} · revision{" "}
              {escalation.revision}, generation {escalation.generation}
              <br />
              Escalation {escalation.id} ·{" "}
              {escalation.source.kind === "task"
                ? `Task ${escalation.source.task_id}`
                : `Communication Run ${escalation.source.run_id}`}
              <br />
              <span className="whitespace-pre-wrap">{escalation.question}</span>
              <br />
              Allowed outcomes: {escalation.allowed_outcomes.join(", ") || "none"}. Assignment:{" "}
              {escalation.latest_assignment
                ? `${escalation.latest_assignment.state.status} (${escalation.latest_assignment.resolver.kind})`
                : "none"}
              .
              <EscalationActions scope={scope} escalation={escalation} />
            </li>
          ))}
        </ul>
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

type CommandState =
  | { kind: "idle" }
  | { kind: "confirm" | "sending" | "unknown"; attempt: ManagementAttempt }
  | { kind: "accepted"; receipt: ManagementReceipt; readback: string | null }
  | { kind: "refused"; message: string };
function ManagementControls({ scope }: { scope: ProjectReadScope }) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const client = useQueryClient();
  const [target, setTarget] = useState<"project" | "task" | "employee">("project");
  const [targetId, setTargetId] = useState("");
  const [mode, setMode] = useState<"graceful" | "force">("graceful");
  const [reason, setReason] = useState("");
  const [waitId, setWaitId] = useState("");
  const [state, setState] = useState<CommandState>({ kind: "idle" });
  const [observed, setObserved] = useState<"project" | "task" | "employee" | null>(null);
  const [observation, setObservation] = useState<string | null>(null);
  const abort = useRef<AbortController | null>(null);
  const confirmation = useRef<HTMLInputElement | null>(null);
  const inFlight = useRef(false);
  useEffect(() => () => abort.current?.abort(), []);
  useEffect(() => {
    if (state.kind !== "confirm" && state.kind !== "sending" && state.kind !== "unknown") return;
    return leaveGuard.register(
      () => true,
      "Leave management action? The prepared retry key will be lost; an in-flight command may still have been applied by Core.",
    );
  }, [leaveGuard, state.kind]);
  const validTarget = UuidV7Schema.safeParse(targetId.trim()).success;
  const projectKey = useMemo(
    () => ["live", generation, projectId, "management-project"],
    [generation, projectId],
  );
  const currentProject = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  const taskKey = useMemo(
    () => ["live", generation, projectId, "management-task", targetId.trim()],
    [generation, projectId, targetId],
  );
  const employeeKey = useMemo(
    () => ["live", generation, projectId, "management-employee", targetId.trim()],
    [generation, projectId, targetId],
  );
  const task = useQuery({
    queryKey: taskKey,
    enabled: target === "task" && validTarget,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.task(projectId, targetId.trim(), token, signal)),
    retry: false,
  });
  const employee = useQuery({
    queryKey: employeeKey,
    enabled: target === "employee" && validTarget,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employee(projectId, targetId.trim(), token, signal),
      ),
    retry: false,
  });
  const detail = target === "task" ? task : employee;
  useReadLifetime(projectKey);
  useReadLifetime(taskKey);
  useReadLifetime(employeeKey);
  const canPrepare =
    !!currentProject.data &&
    (target === "project" || (target === "task" ? !!task.data : !!employee.data)) &&
    !currentProject.isFetching &&
    !detail.isFetching;

  function clear() {
    setState({ kind: "idle" });
    setObserved(null);
    setObservation(null);
  }
  async function prepare(action: ManagementAction) {
    try {
      if (!canPrepare || !currentProject.data) throw Error("Current read unavailable");
      const controller = new AbortController();
      const freshProject = await session.request(generation, (token) =>
        api.project(projectId, token, controller.signal),
      );
      const freshTask =
        target === "task"
          ? await session.request(generation, (token) =>
              api.task(projectId, targetId.trim(), token, controller.signal),
            )
          : null;
      const freshEmployee =
        target === "employee"
          ? await session.request(generation, (token) =>
              api.employee(projectId, targetId.trim(), token, controller.signal),
            )
          : null;
      if (
        action === "resume_task" &&
        !freshTask?.wait_conditions.some((wait) => wait.id === waitId)
      )
        throw Error("Wait is no longer active");
      if (
        action === "pause_task" &&
        (!freshTask || ["draft", "done", "cancelled"].includes(freshTask.lifecycle))
      )
        throw Error("Task cannot be paused");
      if (
        (action === "stop_project_execution" && freshProject.execution_gate !== "open") ||
        (action === "start_project_execution" && freshProject.execution_gate !== "stopped")
      )
        throw Error("Project gate changed");
      const revision = freshProject.revision;
      let payload: unknown;
      if (action === "start_project_execution" || action === "stop_project_execution")
        payload = { reason: reason.trim() || null };
      else if (action === "pause_task")
        payload = {
          task_id: targetId.trim(),
          expected_task_revision: freshTask?.revision,
          mode,
          reason: reason.trim() || null,
        };
      else if (action === "resume_task")
        payload = {
          task_id: targetId.trim(),
          expected_task_revision: freshTask?.revision,
          wait_condition_id: waitId,
        };
      else
        payload = {
          employee_id: targetId.trim(),
          expected_employee_revision: freshEmployee?.revision,
          mode,
          reason: reason.trim() || null,
        };
      const attempt = managementAttempt(action, {
        project_id: projectId,
        expected_revision: revision,
        payload,
      });
      setState({ kind: "confirm", attempt });
    } catch {
      setState({
        kind: "refused",
        message: "Refresh the target and complete the required fields before confirming.",
      });
    }
  }
  async function send(attempt: ManagementAttempt) {
    if (inFlight.current) return;
    inFlight.current = true;
    const controller = new AbortController();
    abort.current = controller;
    setState({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.managementCommand(attempt, token, controller.signal),
      );
      let readback: string | null = null;
      try {
        const project = await session.request(generation, (token) =>
          api.project(projectId, token, controller.signal),
        );
        if (
          attempt.action === "start_project_execution" ||
          attempt.action === "stop_project_execution"
        )
          readback = `Project gate: ${project.execution_gate}; revision ${project.revision}. Physical Run state must be read separately.`;
        else if (attempt.action === "stop_employee") {
          const employee = await session.request(generation, (token) =>
            api.employee(projectId, attempt.resourceId, token, controller.signal),
          );
          readback = `Employee ${employee.id}: ${employee.state}; revision ${employee.revision}. Run stop observations remain separate.`;
        } else {
          const task = await session.request(generation, (token) =>
            api.task(projectId, attempt.resourceId, token, controller.signal),
          );
          readback = `Task ${task.id}: ${task.lifecycle}; revision ${task.revision}; ${task.wait_conditions.length} current waits. Run stop observations remain separate.`;
        }
      } catch {
        readback = "Command saved; canonical readback unavailable. Refresh before another action.";
      }
      setState({ kind: "accepted", receipt, readback });
      setObserved(target);
      setObservation(readback);
      void client.invalidateQueries({ queryKey: ["live", generation, projectId] });
      void currentProject.refetch();
      if (target === "task") void task.refetch();
      if (target === "employee") void employee.refetch();
    } catch (error) {
      if (!(error instanceof LiveCommandError) || error.kind === "outcome_unknown")
        setState({ kind: "unknown", attempt });
      else
        setState({
          kind: "refused",
          message: `Core refused ${attempt.action}: ${error.kind}. Refresh target and Project before preparing again.`,
        });
    } finally {
      inFlight.current = false;
      abort.current = null;
    }
  }
  return (
    <section
      aria-label="Management controls"
      className="space-y-3 rounded-xl border border-border p-4"
    >
      <h3 className="font-medium">Scoped controls</h3>
      <p className="text-xs text-muted-foreground">
        A confirmed command records management intent. Force revokes logical authority; physical
        reservations stay until observed release.
      </p>
      <label className="block">
        Target{" "}
        <select
          value={target}
          disabled={
            state.kind === "confirm" || state.kind === "sending" || state.kind === "unknown"
          }
          onChange={(event) => {
            setTarget(event.target.value as typeof target);
            clear();
          }}
          className="rounded border p-2"
        >
          <option value="project">Project</option>
          <option value="task">Task</option>
          <option value="employee">Employee</option>
        </select>
      </label>
      {target !== "project" && (
        <label className="block">
          {target === "task" ? "Task" : "Employee"} ID
          <Input
            value={targetId}
            disabled={
              state.kind === "confirm" || state.kind === "sending" || state.kind === "unknown"
            }
            onChange={(event) => {
              setTargetId(event.target.value);
              clear();
            }}
          />
        </label>
      )}
      {currentProject.isPending && <p role="status">Loading Project revision…</p>}
      {currentProject.isError && <p role="alert">{describeApiError(currentProject.error)}</p>}
      {target !== "project" && validTarget && detail.isPending && (
        <p role="status">Loading target…</p>
      )}
      {target !== "project" && validTarget && detail.isError && (
        <p role="alert">
          {describeApiError(detail.error, target === "task" ? "Task" : "Employee")}
        </p>
      )}
      {currentProject.data && target === "project" && (
        <p className="text-sm [overflow-wrap:anywhere]">
          Current: {currentProject.data.id} · revision {currentProject.data.revision} · gate{" "}
          {currentProject.data.execution_gate}
        </p>
      )}
      {target === "task" && task.data && (
        <p className="text-sm [overflow-wrap:anywhere]">
          Current: {task.data.id} · revision {task.data.revision} · lifecycle {task.data.lifecycle}
        </p>
      )}
      {target === "employee" && employee.data && (
        <p className="text-sm [overflow-wrap:anywhere]">
          Current: {employee.data.id} · revision {employee.data.revision} · state{" "}
          {employee.data.state}
        </p>
      )}
      {(target === "task" || target === "employee") && (
        <label className="block">
          Stop mode{" "}
          <select
            value={mode}
            disabled={
              state.kind === "confirm" || state.kind === "sending" || state.kind === "unknown"
            }
            onChange={(event) => setMode(event.target.value as typeof mode)}
            className="rounded border p-2"
          >
            <option value="graceful">Graceful</option>
            <option value="force">Force logical authority</option>
          </select>
        </label>
      )}
      <label className="block">
        Reason (optional)
        <Input
          value={reason}
          disabled={
            state.kind === "confirm" || state.kind === "sending" || state.kind === "unknown"
          }
          onChange={(event) => setReason(event.target.value)}
          maxLength={10000}
        />
      </label>
      {target === "task" && task.data && (
        <label className="block">
          Wait to resume
          <select
            value={waitId}
            disabled={
              state.kind === "confirm" || state.kind === "sending" || state.kind === "unknown"
            }
            onChange={(event) => setWaitId(event.target.value)}
            className="max-w-full rounded border p-2"
          >
            <option value="">Select an active wait</option>
            {task.data.wait_conditions.map((wait) => (
              <option key={wait.id} value={wait.id}>
                {wait.kind} · {wait.id}
              </option>
            ))}
          </select>
        </label>
      )}
      {state.kind !== "confirm" && state.kind !== "unknown" && (
        <div className="flex flex-wrap gap-2">
          {target === "project" &&
            currentProject.data &&
            (currentProject.data.execution_gate === "open" ? (
              <Button
                variant="outline"
                disabled={!canPrepare}
                onClick={() => void prepare("stop_project_execution")}
              >
                Prepare Project stop
              </Button>
            ) : (
              <Button
                variant="outline"
                disabled={!canPrepare}
                onClick={() => void prepare("start_project_execution")}
              >
                Prepare Project start
              </Button>
            ))}
          {target === "task" && task.data && (
            <>
              <Button
                variant="outline"
                disabled={
                  !canPrepare || ["draft", "done", "cancelled"].includes(task.data.lifecycle)
                }
                onClick={() => void prepare("pause_task")}
              >
                Prepare Task pause
              </Button>
              <Button
                variant="outline"
                disabled={!canPrepare || !waitId}
                onClick={() => void prepare("resume_task")}
              >
                Prepare resolve wait
              </Button>
            </>
          )}
          {target === "employee" && employee.data && (
            <Button
              variant="outline"
              disabled={!canPrepare}
              onClick={() => void prepare("stop_employee")}
            >
              Prepare Employee stop
            </Button>
          )}
        </div>
      )}
      {state.kind === "confirm" && (
        <div className="space-y-2 rounded border p-3">
          <p>
            Confirm {state.attempt.action.replaceAll("_", " ")} for {state.attempt.resourceId}.
            Project revision {state.attempt.expectedRevision}. This may request stops; it does not
            prove physical quiescence.
          </p>
          <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] text-xs">
            {state.attempt.body}
          </pre>
          <label className="flex gap-2">
            <input type="checkbox" ref={confirmation} /> I confirm this exact action
          </label>
          <Button
            onClick={() => {
              if (confirmation.current?.checked) void send(state.attempt);
            }}
          >
            Send confirmed command
          </Button>
          <Button variant="outline" onClick={clear}>
            Cancel
          </Button>
        </div>
      )}
      {state.kind === "sending" && <p role="status">Saving management command…</p>}
      {state.kind === "unknown" && (
        <div role="alert">
          Outcome unknown. Retry the identical command and key after checking the current state.
          <Button variant="outline" onClick={() => void send(state.attempt)}>
            Retry exact request
          </Button>
        </div>
      )}
      {state.kind === "refused" && <p role="alert">{state.message}</p>}
      {state.kind === "accepted" && (
        <p role="status">
          Core saved command ({state.receipt.status}); receipt {state.receipt.command_id}.{" "}
          {state.readback}
        </p>
      )}
      {observed && observation && (
        <p className="sr-only">
          Last readback for {observed}: {observation}
        </p>
      )}
    </section>
  );
}
