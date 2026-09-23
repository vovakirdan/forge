import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  resolverAttempt,
  type ResolverAction,
  type ResolverAttempt,
} from "../contracts/resolver-command.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendResolverCommand } from "./resolver-command-api.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type State =
  | { kind: "editing" }
  | {
      kind: "confirm" | "sending" | "unknown";
      attempt: ResolverAttempt;
      oldRouteRevision: number | null;
    }
  | { kind: "refused"; error: LiveCommandError }
  | {
      kind: "accepted";
      attempt: ResolverAttempt;
      oldRouteRevision: number | null;
      id: string;
      readError: boolean;
    };

export function ResolverActions({ scope }: { scope: ProjectReadScope }) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [action, setAction] = useState<ResolverAction>("configure_resolver_route");
  const [routeKey, setRouteKey] = useState("");
  const [employeeIds, setEmployeeIds] = useState("");
  const [timeout, setTimeoutValue] = useState("300");
  const [taskId, setTaskId] = useState("");
  const [category, setCategory] = useState("clarification");
  const [question, setQuestion] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [state, setState] = useState<State>({ kind: "editing" });
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const projectKey = useMemo(
    () => ["live", generation, projectId, "resolver-command-project"] as const,
    [generation, projectId],
  );
  const project = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(projectKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    return () => next.abort();
  }, []);
  const dirty = !!(
    routeKey ||
    employeeIds ||
    taskId ||
    question ||
    state.kind === "sending" ||
    state.kind === "unknown"
  );
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave resolver action? Inputs and the retry key will be lost. An in-flight command may have been applied.",
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
  const locked = ["confirm", "sending", "unknown", "accepted"].includes(state.kind);
  function current(next: AbortController) {
    return !next.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function findRoute(key: string, signal: AbortSignal) {
    let cursor: string | null = null;
    for (let page = 0; page < 20; page += 1) {
      const response = await session.request(generation, (token) =>
        api.managementFactsPage(projectId, "resolver-routes", cursor, token, signal),
      );
      if (response.kind !== "resolver-routes") throw Error("Wrong route response");
      const found = response.value.items.find((item) => item.key === key);
      if (found) return found;
      if (!response.value.next_cursor) return null;
      if (page === 19) throw Error("Route page bound reached");
      cursor = response.value.next_cursor;
    }
    return null;
  }
  async function readAccepted(
    attempt: ResolverAttempt,
    oldRouteRevision: number | null,
    id: string,
  ) {
    const next = controller.current;
    if (!next) return;
    setState({ kind: "accepted", attempt, oldRouteRevision, id, readError: false });
    try {
      const fresh = await session.request(generation, (token) =>
        api.project(projectId, token, next.signal),
      );
      if (fresh.revision < attempt.expectedRevision + 1) throw Error("Project readback absent");
      const payload = (
        JSON.parse(attempt.body) as {
          payload: {
            route_key?: string | null;
            task_id?: string;
            category?: string;
            question?: string;
            employee_ids?: string[];
            assignment_timeout_seconds?: number;
          };
        }
      ).payload;
      if (attempt.action === "configure_resolver_route") {
        const route = await findRoute(attempt.routeKey!, next.signal);
        if (
          !route ||
          route.revision !== (oldRouteRevision ?? 0) + 1 ||
          route.assignment_timeout_seconds !== payload.assignment_timeout_seconds ||
          JSON.stringify(route.employee_ids) !== JSON.stringify(payload.employee_ids)
        )
          throw Error("Route readback absent");
      } else {
        const escalation = await session.request(generation, (token) =>
          api.escalationDetail(projectId, id, token, next.signal),
        );
        if (
          escalation.source.kind !== "task" ||
          escalation.source.task_id.toLowerCase() !== attempt.taskId?.toLowerCase() ||
          escalation.category !== payload.category ||
          escalation.question !== payload.question
        )
          throw Error("Escalation readback absent");
      }
      if (!current(next)) return;
      queries.setQueryData(["project", generation, projectId], fresh);
      void queries.invalidateQueries({ queryKey: projectKey });
      void queries.invalidateQueries({ queryKey: ["live", generation, projectId, "management"] });
      setRouteKey("");
      setEmployeeIds("");
      setTimeoutValue("300");
      setTaskId("");
      setQuestion("");
      setState({ kind: "editing" });
    } catch {
      if (current(next))
        setState({ kind: "accepted", attempt, oldRouteRevision, id, readError: true });
    }
  }
  async function send(attempt: ResolverAttempt, oldRouteRevision: number | null) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt, oldRouteRevision });
    try {
      const receipt = await session.request(generation, (token) =>
        sendResolverCommand(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt, oldRouteRevision, receipt.resource.id);
    } catch (cause) {
      if (current(next))
        setState(
          cause instanceof LiveCommandError && cause.kind !== "outcome_unknown"
            ? { kind: "refused", error: cause }
            : { kind: "unknown", attempt, oldRouteRevision },
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
      let payload: unknown;
      let oldRouteRevision: number | null = null;
      if (action === "configure_resolver_route") {
        const employees = employeeIds.split(/[\s,]+/).filter(Boolean);
        for (const id of employees)
          await session.request(generation, (token) =>
            api.employee(projectId, id, token, next.signal),
          );
        oldRouteRevision = (await findRoute(routeKey.trim(), next.signal))?.revision ?? null;
        payload = {
          route_key: routeKey.trim(),
          employee_ids: employees,
          assignment_timeout_seconds: Number(timeout),
        };
      } else {
        const task = await session.request(generation, (token) =>
          api.task(projectId, taskId.trim(), token, next.signal),
        );
        if (routeKey.trim() && !(await findRoute(routeKey.trim(), next.signal)))
          throw Error("Route missing");
        payload = {
          task_id: taskId.trim(),
          expected_task_revision: task.revision,
          route_key: routeKey.trim() || null,
          category,
          question,
        };
      }
      const attempt = resolverAttempt(action, {
        project_id: projectId,
        expected_revision: fresh.revision,
        payload,
      });
      if (current(next)) {
        setError(null);
        setState({ kind: "confirm", attempt, oldRouteRevision });
      }
    } catch {
      if (current(next))
        setError(
          "Current Project, Task, route or Employee could not be verified. Refresh and check the fields.",
        );
    }
  }
  return (
    <section
      aria-label="Resolver route and escalation"
      className="space-y-3 rounded-xl border border-border p-4"
    >
      <h3 className="font-medium">Resolver route and escalation</h3>
      <p className="text-xs text-muted-foreground">
        Routes contain ordered Employee candidates with Human fallback. Raising an escalation adds a
        Task wait and requests current Run stop.
      </p>
      {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
      <label className="block text-sm">
        Action
        <select
          className="mt-1 block w-full rounded border border-border bg-background p-2"
          value={action}
          disabled={locked}
          onChange={(event) => {
            setAction(event.target.value as ResolverAction);
            setState({ kind: "editing" });
            setError(null);
          }}
        >
          <option value="configure_resolver_route">Configure resolver route</option>
          <option value="raise_escalation">Raise Task escalation</option>
        </select>
      </label>
      <label className="block text-sm">
        Route key {action === "raise_escalation" ? "(optional)" : ""}
        <Input
          value={routeKey}
          disabled={locked}
          onChange={(event) => setRouteKey(event.target.value)}
        />
      </label>
      {action === "configure_resolver_route" ? (
        <>
          <label className="block text-sm">
            Employee IDs in priority order (comma or newline separated)
            <textarea
              className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
              rows={3}
              value={employeeIds}
              disabled={locked}
              onChange={(event) => setEmployeeIds(event.target.value)}
            />
          </label>
          <label className="block text-sm">
            Assignment timeout seconds
            <Input
              type="number"
              min={30}
              max={86400}
              value={timeout}
              disabled={locked}
              onChange={(event) => setTimeoutValue(event.target.value)}
            />
          </label>
        </>
      ) : (
        <>
          <label className="block text-sm">
            Task ID
            <Input
              value={taskId}
              disabled={locked}
              onChange={(event) => setTaskId(event.target.value)}
            />
          </label>
          <label className="block text-sm">
            Category
            <select
              className="mt-1 block w-full rounded border border-border bg-background p-2"
              value={category}
              disabled={locked}
              onChange={(event) => setCategory(event.target.value)}
            >
              <option value="action_approval">Action approval</option>
              <option value="clarification">Clarification</option>
              <option value="scope_or_policy_conflict">Scope or policy conflict</option>
              <option value="stale_or_invalid_task">Stale or invalid Task</option>
              <option value="technical_decision">Technical decision</option>
              <option value="blocked">Blocked</option>
            </select>
          </label>
          <label className="block text-sm">
            Question
            <textarea
              className="mt-1 w-full rounded border border-border bg-background p-2 text-sm"
              rows={4}
              value={question}
              disabled={locked}
              onChange={(event) => setQuestion(event.target.value)}
            />
          </label>
        </>
      )}
      {error && <p role="alert">{error}</p>}
      {(state.kind === "editing" || state.kind === "refused") && (
        <Button disabled={!project.data || project.isFetching} onClick={() => void prepare()}>
          Review resolver action
        </Button>
      )}
      {state.kind === "confirm" && (
        <div
          role="alertdialog"
          aria-label="Confirm resolver action"
          className="space-y-2 rounded border border-border p-3"
        >
          <p>Confirm {state.attempt.action.replaceAll("_", " ")}?</p>
          <Button onClick={() => void send(state.attempt, state.oldRouteRevision)}>Confirm</Button>
          <Button variant="outline" onClick={() => setState({ kind: "editing" })}>
            Cancel
          </Button>
        </div>
      )}
      {state.kind === "sending" && <p role="status">Saving resolver action…</p>}
      {state.kind === "unknown" && (
        <div className="space-y-2">
          <p role="alert">Outcome unknown. Retry the exact same request and key.</p>
          <Button onClick={() => void send(state.attempt, state.oldRouteRevision)}>
            Retry same request
          </Button>
        </div>
      )}
      {state.kind === "refused" && (
        <p role="alert">
          Core refused action ({state.error.kind}). Inputs are preserved; review the current facts
          and revisions.
        </p>
      )}
      {state.kind === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Command accepted.{" "}
            {state.readError ? "Canonical readback failed." : "Reading canonical result…"}
          </p>
          {state.readError && (
            <Button
              onClick={() => void readAccepted(state.attempt, state.oldRouteRevision, state.id)}
            >
              Retry readback
            </Button>
          )}
        </div>
      )}
    </section>
  );
}
