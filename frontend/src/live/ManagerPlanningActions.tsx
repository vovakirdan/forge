import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { UuidV7Schema } from "../contracts/common.ts";
import {
  managerPlanningAttempt,
  type ManagerPlanningAction,
  type ManagerPlanningAttempt,
} from "../contracts/manager-planning.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendManagerPlanning } from "./manager-planning-api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type State =
  | { kind: "editing" }
  | { kind: "confirm" | "sending" | "unknown"; attempt: ManagerPlanningAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; attempt: ManagerPlanningAttempt; receiptId: string; readError: boolean };

export function ManagerPlanningActions({ scope }: { scope: ProjectReadScope }) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [action, setAction] = useState<ManagerPlanningAction>("set_next_run_employee");
  const [taskId, setTaskId] = useState("");
  const [employeeId, setEmployeeId] = useState("");
  const [waitId, setWaitId] = useState("");
  const [scheduleId, setScheduleId] = useState("");
  const [notBefore, setNotBefore] = useState("");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [state, setState] = useState<State>({ kind: "editing" });
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const projectKey = useMemo(
    () => ["live", generation, projectId, "manager-planning-project"] as const,
    [generation, projectId],
  );
  const project = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(projectKey);
  const taskKey = useMemo(
    () => ["live", generation, projectId, "manager-planning-task", taskId] as const,
    [generation, projectId, taskId],
  );
  const task = useQuery({
    queryKey: taskKey,
    enabled: action !== "cancel_task_resume" && UuidV7Schema.safeParse(taskId).success,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.task(projectId, taskId, token, signal)),
    retry: false,
  });
  useReadLifetime(taskKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    return () => next.abort();
  }, []);
  const dirty = !!(
    taskId ||
    employeeId ||
    waitId ||
    scheduleId ||
    notBefore ||
    reason ||
    state.kind === "unknown" ||
    state.kind === "sending"
  );
  const locked = ["confirm", "sending", "unknown", "accepted"].includes(state.kind);
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave management planning? Unsaved inputs and the retry key will be lost. A command in flight may have been applied.",
    );
    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [dirty, leaveGuard]);
  function current(next: AbortController) {
    return !next.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function findConstraint(
    predicate: (item: {
      id: string;
      task_id: string;
      employee_id: string;
      state: { status: string };
    }) => boolean,
    signal: AbortSignal,
  ) {
    let cursor: string | null = null;
    for (let page = 0; page < 20; page += 1) {
      const result = await session.request(generation, (token) =>
        api.managementFactsPage(projectId, "next-run-constraints", cursor, token, signal),
      );
      if (result.kind !== "next-run-constraints") throw Error("wrong facts response");
      const found = result.value.items.find(predicate);
      if (found) return found;
      if (!result.value.next_cursor) break;
      if (page === 19) throw Error("Constraint readback page bound reached");
      cursor = result.value.next_cursor;
    }
    return null;
  }
  async function findSchedule(id: string, signal: AbortSignal) {
    let cursor: string | null = null;
    for (let page = 0; page < 20; page += 1) {
      const result = await session.request(generation, (token) =>
        api.managementPage(projectId, "resume-schedules", cursor, token, signal),
      );
      if (result.kind !== "resume-schedules") throw Error("wrong schedule response");
      const found = result.value.items.find((item) => item.id.toLowerCase() === id.toLowerCase());
      if (found) return found;
      if (!result.value.next_cursor) break;
      if (page === 19) throw Error("Schedule readback page bound reached");
      cursor = result.value.next_cursor;
    }
    return null;
  }
  async function readAccepted(attempt: ManagerPlanningAttempt, receiptId: string) {
    const next = controller.current;
    if (!next) return;
    setState({ kind: "accepted", attempt, receiptId, readError: false });
    try {
      const fresh = await session.request(generation, (token) =>
        api.project(projectId, token, next.signal),
      );
      if (fresh.revision < attempt.expectedRevision + 1)
        throw Error("Project revision readback absent");
      const payload = (JSON.parse(attempt.body) as { payload: Record<string, string> }).payload;
      const taskId = payload["task_id"];
      const employeeId = payload["employee_id"];
      if (attempt.action === "schedule_task_resume" || attempt.action === "cancel_task_resume") {
        const row = await findSchedule(receiptId, next.signal);
        if (
          !row ||
          (attempt.action === "schedule_task_resume"
            ? row.state.status !== "pending" ||
              !taskId ||
              row.task_id.toLowerCase() !== taskId.toLowerCase()
            : row.state.status !== "cancelled")
        )
          throw Error("Schedule readback absent");
      } else {
        if (attempt.action === "set_next_run_employee") {
          const row = await findConstraint(
            (item) =>
              !!taskId &&
              !!employeeId &&
              item.task_id.toLowerCase() === taskId.toLowerCase() &&
              item.employee_id.toLowerCase() === employeeId.toLowerCase() &&
              ["pending", "blocked"].includes(item.state.status),
            next.signal,
          );
          if (!row) throw Error("Constraint readback absent");
        } else {
          const active = await findConstraint(
            (item) =>
              !!taskId &&
              item.task_id.toLowerCase() === taskId.toLowerCase() &&
              ["pending", "blocked"].includes(item.state.status),
            next.signal,
          );
          if (active) throw Error("Constraint remains active");
        }
      }
      if (!current(next)) return;
      queries.setQueryData(["project", generation, projectId], fresh);
      void queries.invalidateQueries({ queryKey: projectKey });
      void queries.invalidateQueries({ queryKey: ["live", generation, projectId, "management"] });
      setTaskId("");
      setEmployeeId("");
      setWaitId("");
      setScheduleId("");
      setNotBefore("");
      setReason("");
      setState({ kind: "editing" });
    } catch {
      if (current(next)) setState({ kind: "accepted", attempt, receiptId, readError: true });
    }
  }
  async function send(attempt: ManagerPlanningAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendManagerPlanning(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt, receipt.resource.id);
    } catch (cause) {
      if (current(next))
        setState(
          cause instanceof LiveCommandError && cause.kind !== "outcome_unknown"
            ? { kind: "refused", error: cause }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  async function prepare() {
    const next = controller.current;
    if (!next) return;
    try {
      const fresh = await session.request(generation, (token) =>
        api.project(projectId, token, next.signal),
      );
      const currentTask =
        action === "cancel_task_resume"
          ? null
          : await session.request(generation, (token) =>
              api.task(projectId, taskId.trim(), token, next.signal),
            );
      let payload: Record<string, unknown>;
      if (action === "set_next_run_employee") {
        await session.request(generation, (token) =>
          api.employee(projectId, employeeId.trim(), token, next.signal),
        );
        payload = {
          task_id: taskId.trim(),
          expected_task_revision: currentTask?.revision,
          employee_id: employeeId.trim(),
          reason: reason.trim() || null,
        };
      } else if (action === "clear_next_run_employee") {
        const row = await findConstraint(
          (item) =>
            item.task_id.toLowerCase() === taskId.trim().toLowerCase() &&
            ["pending", "blocked"].includes(item.state.status),
          next.signal,
        );
        if (!row) throw Error("No current next-Run constraint for this Task");
        payload = {
          task_id: taskId.trim(),
          expected_task_revision: currentTask?.revision,
          reason: reason.trim() || null,
        };
      } else if (action === "schedule_task_resume") {
        if (
          !currentTask?.wait_conditions.some(
            (wait) => wait.id.toLowerCase() === waitId.trim().toLowerCase(),
          )
        )
          throw Error("Wait no longer exists");
        const due = new Date(notBefore);
        if (!Number.isFinite(due.getTime()) || due.getTime() <= Date.now())
          throw Error("Choose a future date and time");
        payload = {
          task_id: taskId.trim(),
          expected_task_revision: currentTask.revision,
          wait_condition_id: waitId.trim(),
          not_before: due.toISOString(),
          reason: reason.trim(),
        };
      } else {
        const row = await findSchedule(scheduleId.trim(), next.signal);
        if (!row || row.state.status !== "pending") throw Error("Schedule is no longer pending");
        payload = { schedule_id: scheduleId.trim(), reason: reason.trim() || null };
      }
      const attempt = managerPlanningAttempt(action, {
        project_id: projectId,
        expected_revision: fresh.revision,
        payload,
      });
      if (current(next)) {
        setError(null);
        setState({ kind: "confirm", attempt });
      }
    } catch {
      if (current(next))
        setError(
          "Current Project, Task, Employee, wait or schedule could not be verified. Refresh and check the fields.",
        );
    }
  }
  return (
    <section
      aria-label="Next Run and resume planning"
      className="space-y-3 rounded-xl border border-border p-4"
    >
      <h3 className="font-medium">Next Run and resume planning</h3>
      <p className="text-xs text-muted-foreground">
        These commands record future intent. They do not transfer active work or promise a Run at
        the deadline.
      </p>
      {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
      <label className="block text-sm">
        Action
        <select
          className="mt-1 block w-full rounded border border-border bg-background p-2"
          value={action}
          disabled={locked}
          onChange={(event) => {
            setAction(event.target.value as ManagerPlanningAction);
            setState({ kind: "editing" });
            setError(null);
          }}
        >
          <option value="set_next_run_employee">Set next-Run Employee</option>
          <option value="clear_next_run_employee">Clear next-Run Employee</option>
          <option value="schedule_task_resume">Schedule Task resume</option>
          <option value="cancel_task_resume">Cancel scheduled resume</option>
        </select>
      </label>
      {action !== "cancel_task_resume" && (
        <label className="block text-sm">
          Task ID
          <Input
            value={taskId}
            onChange={(event) => setTaskId(event.target.value)}
            disabled={locked}
          />
        </label>
      )}
      {action === "set_next_run_employee" && (
        <label className="block text-sm">
          Employee ID
          <Input
            value={employeeId}
            onChange={(event) => setEmployeeId(event.target.value)}
            disabled={locked}
          />
        </label>
      )}
      {action === "schedule_task_resume" && (
        <>
          <label className="block text-sm">
            Wait condition ID
            <Input
              value={waitId}
              onChange={(event) => setWaitId(event.target.value)}
              disabled={locked}
            />
          </label>
          <label className="block text-sm">
            Not before
            <input
              type="datetime-local"
              className="mt-1 block rounded border border-border bg-background p-2"
              value={notBefore}
              onChange={(event) => setNotBefore(event.target.value)}
              disabled={locked}
            />
          </label>
        </>
      )}
      {action === "cancel_task_resume" && (
        <label className="block text-sm">
          Schedule ID
          <Input
            value={scheduleId}
            onChange={(event) => setScheduleId(event.target.value)}
            disabled={locked}
          />
        </label>
      )}
      <label className="block text-sm">
        Reason {action === "schedule_task_resume" ? "(required)" : "(optional)"}
        <Input
          value={reason}
          onChange={(event) => setReason(event.target.value)}
          disabled={locked}
        />
      </label>
      {task.isError && action !== "cancel_task_resume" && (
        <p role="alert">{describeApiError(task.error, "Task")}</p>
      )}
      {error && <p role="alert">{error}</p>}
      {(state.kind === "editing" || state.kind === "refused") && (
        <Button disabled={!project.data || project.isFetching} onClick={() => void prepare()}>
          Review action
        </Button>
      )}
      {state.kind === "confirm" && (
        <div
          role="alertdialog"
          aria-label="Confirm management planning"
          className="space-y-2 rounded border border-border p-3"
        >
          <p>Confirm {state.attempt.action.replaceAll("_", " ")} for this Project?</p>
          <Button onClick={() => void send(state.attempt)}>Confirm</Button>
          <Button variant="outline" onClick={() => setState({ kind: "editing" })}>
            Cancel
          </Button>
        </div>
      )}
      {state.kind === "sending" && <p role="status">Saving management intent…</p>}
      {state.kind === "unknown" && (
        <div className="space-y-2">
          <p role="alert">Outcome unknown. Retry the exact same request and key.</p>
          <Button onClick={() => void send(state.attempt)}>Retry same request</Button>
        </div>
      )}
      {state.kind === "refused" && (
        <p role="alert">
          Core refused action ({state.error.kind}). Inputs are preserved; refresh and review
          revisions before trying again.
        </p>
      )}
      {state.kind === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Command accepted.{" "}
            {state.readError ? "Canonical readback failed." : "Reading saved intent…"}
          </p>
          {state.readError && (
            <Button onClick={() => void readAccepted(state.attempt, state.receiptId)}>
              Retry readback
            </Button>
          )}
        </div>
      )}
    </section>
  );
}
