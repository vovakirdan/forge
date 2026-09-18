import { startTransition, useActionState, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { TaskCommandAttempt, TaskCommandReceipt } from "../contracts/task-command.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { describeCancellationError } from "./cancellation-error.ts";
import { CancellationNoteSchema } from "../contracts/cancel-task.ts";
import {
  createCancellationAttempt,
  canCancelTask,
  type CancellationBaseline,
} from "./cancellation-attempt.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { readKeys, invalidateProjectDependencies } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type CancellationProps = ProjectReadScope & {
  taskId: string;
  onCancel: () => void;
};
async function readBaseline(
  scope: CancellationProps,
  signal: AbortSignal,
): Promise<CancellationBaseline> {
  const { session, generation, api, projectId, taskId } = scope;
  const [project, task] = await Promise.all([
    session.request(generation, (token) => api.project(projectId, token, signal)),
    session.request(generation, (token) => api.task(projectId, taskId, token, signal)),
  ]);
  const catalog = await session.request(generation, (token) =>
    api.cancellationReasons(projectId, token, signal),
  );
  if (project.revision !== catalog.project_revision)
    throw new Error("Project changed while loading cancellation baseline");
  return { project, task, catalog };
}

export function TaskCancellationPanel(scope: CancellationProps) {
  const key = useMemo(
    () =>
      ["live", scope.generation, scope.projectId, "cancellation-baseline", scope.taskId] as const,
    [scope.generation, scope.projectId, scope.taskId],
  );
  const baseline = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => readBaseline(scope, signal),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section aria-label="Cancel task" className="space-y-3 rounded-lg border border-border p-4">
      <h3 className="font-semibold">Cancel task</h3>
      {baseline.isPending && <p role="status">Loading current cancellation baseline…</p>}
      {baseline.isError && (
        <>
          <p role="alert">
            Cannot load the cancellation baseline. {describeApiError(baseline.error, "Task")}
          </p>
          <Button disabled={baseline.isFetching} onClick={() => void baseline.refetch()}>
            Retry cancellation baseline
          </Button>
        </>
      )}
      {baseline.data ? (
        <CancellationForm {...scope} initial={baseline.data} />
      ) : (
        <Button variant="outline" onClick={scope.onCancel}>
          Cancel
        </Button>
      )}
    </section>
  );
}

type CancellationState =
  | { status: "confirming" }
  | { status: "sending" | "unknown"; attempt: TaskCommandAttempt }
  | { status: "refused"; error: LiveCommandError }
  | {
      status: "cancelled";
      receipt: TaskCommandReceipt;
      refreshing: boolean;
      refreshFailed: boolean;
    };

function CancellationForm(scope: CancellationProps & { initial: CancellationBaseline }) {
  const { api, session, generation, projectId, taskId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [baseline, setBaseline] = useState(scope.initial);
  const [confirmed, setConfirmed] = useState(false);
  const [reason, setReason] = useState("");
  const [note, setNote] = useState("");
  const [state, setState] = useState<CancellationState>({ status: "confirming" });
  const [refreshing, setRefreshing] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  const warn =
    state.status === "sending" ||
    state.status === "unknown" ||
    (state.status !== "cancelled" && (confirmed || reason !== "" || note !== ""));
  useEffect(() => {
    if (!warn) return;
    const unregister = leaveGuard.register(() => true);
    function beforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "";
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [leaveGuard, warn]);
  function stillCurrent(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function refreshCancelled(receipt: TaskCommandReceipt, controller: AbortController) {
    setState({ status: "cancelled", receipt, refreshing: true, refreshFailed: false });
    // Invalidate on the receipt, even when the independent Task readback fails.
    void invalidateProjectDependencies(queries, generation, projectId);
    try {
      // A cancellation receipt does not prove observed Run termination.
      const [project, task] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
        session.request(generation, (token) =>
          api.task(projectId, taskId, token, controller.signal),
        ),
      ]);
      if (!stillCurrent(controller)) return;
      const projectKey = ["project", generation, projectId] as const;
      const taskKey = readKeys.task(generation, projectId, taskId);
      await Promise.all(
        [projectKey, taskKey].map((queryKey) => queries.cancelQueries({ queryKey, exact: true })),
      );
      if (!stillCurrent(controller)) return;
      queries.setQueryData(projectKey, project);
      queries.setQueryData(taskKey, task);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "tasks"] },
        { throwOnError: true },
      );
      if (stillCurrent(controller))
        setState({ status: "cancelled", receipt, refreshing: false, refreshFailed: false });
    } catch {
      if (stillCurrent(controller))
        setState({ status: "cancelled", receipt, refreshing: false, refreshFailed: true });
    }
  }
  async function submit(attempt: TaskCommandAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.cancelTask(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (stillCurrent(controller)) await refreshCancelled(receipt, controller);
    } catch (error) {
      if (!stillCurrent(controller)) return;
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
    if (!controller || refreshing || state.status !== "refused") return;
    setRefreshing(true);
    setReadError(null);
    setConfirmed(false);
    try {
      const current = await readBaseline(scope, controller.signal);
      if (!stillCurrent(controller)) return;
      setBaseline(current);
      setState({ status: "confirming" });
    } catch (error) {
      if (stillCurrent(controller)) setReadError(describeApiError(error, "Task"));
    } finally {
      if (stillCurrent(controller)) setRefreshing(false);
    }
  }
  const active = baseline.catalog.reasons.some((entry) => entry.id === reason && !entry.retired);
  const cancellable = canCancelTask(baseline.task.lifecycle);
  const validNote =
    /^\p{White_Space}*$/u.test(note) || CancellationNoteSchema.safeParse(note).success;
  const revisionSupported =
    baseline.project.revision < Number.MAX_SAFE_INTEGER &&
    baseline.task.revision < Number.MAX_SAFE_INTEGER;
  const confirmable =
    state.status === "confirming" && active && cancellable && validNote && revisionSupported;
  const [, cancel, actionPending] = useActionState(async (_previous: null) => {
    if (confirmable && confirmed) await submit(createCancellationAttempt(baseline, reason, note));
    return null;
  }, null);
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        startTransition(cancel);
      }}
      className="space-y-3"
    >
      <p className="text-sm">
        Cancellation closes the Task and asks Core to stop active Runs gracefully. It does not prove
        that processes have physically stopped.
      </p>
      <p className="text-sm [overflow-wrap:anywhere]">Saved title: {baseline.task.title}</p>
      <p className="text-xs">
        Cancellation baseline: Project revision {baseline.project.revision}, Task revision{" "}
        {baseline.task.revision}.
      </p>
      {!cancellable && <p role="alert">This Task is terminal. Cancellation is unavailable.</p>}
      {!revisionSupported && (
        <p role="alert">
          The revision exceeds the browser command limit. Cancellation is unavailable here.
        </p>
      )}
      <label className="block space-y-1">
        <span>Cancellation reason</span>
        <select
          className="block w-full rounded border border-border bg-background p-2"
          value={reason}
          disabled={state.status !== "confirming" || actionPending || !cancellable}
          onChange={(event) => {
            setReason(event.target.value);
            setConfirmed(false);
          }}
        >
          <option value="">Choose a reason</option>
          {reason !== "" && !active && (
            <option value={reason} disabled>
              {reason} (no longer active)
            </option>
          )}
          {baseline.catalog.reasons
            .filter((entry) => !entry.retired)
            .map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.display_name} ({entry.id})
              </option>
            ))}
        </select>
      </label>
      {reason !== "" && !active && (
        <p role="alert">The selected reason is no longer active. Choose an active reason.</p>
      )}
      <label className="block space-y-1">
        <span>Cancellation note (optional)</span>
        <textarea
          className="block w-full rounded border border-border bg-background p-2"
          value={note}
          disabled={state.status !== "confirming" || actionPending || !cancellable}
          onChange={(event) => {
            setNote(event.target.value);
            setConfirmed(false);
          }}
        />
      </label>
      {!validNote && (
        <p role="alert">The note must contain valid Unicode text and at most 20,000 characters.</p>
      )}
      {state.status !== "cancelled" && (
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={!confirmable || actionPending}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          I understand cancellation requests Run shutdown
        </label>
      )}
      {state.status === "sending" && <p role="status">Cancelling task…</p>}
      {state.status === "unknown" && (
        <>
          <p role="alert">{describeCancellationError(new LiveCommandError("outcome_unknown"))}</p>
          <Button type="button" disabled={actionPending} onClick={() => void submit(state.attempt)}>
            Retry same cancellation
          </Button>
        </>
      )}
      {state.status === "refused" && (
        <>
          <p role="alert">{describeCancellationError(state.error)}</p>
          <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
            Refresh cancellation baseline
          </Button>
        </>
      )}
      {readError && <p role="alert">{readError}</p>}
      {state.status === "cancelled" && (
        <>
          <p role="status">
            Cancellation accepted.{" "}
            {state.refreshing
              ? "Refreshing authoritative data…"
              : state.refreshFailed
                ? "Could not refresh authoritative data. Cancellation is confirmed."
                : "Authoritative data refreshed."}
          </p>
          <p className="text-xs">
            Receipt: {state.receipt.command_id} ({state.receipt.status})
          </p>
          {state.refreshFailed && (
            <Button
              type="button"
              onClick={() => {
                const controller = lifetime.current;
                if (controller) void refreshCancelled(state.receipt, controller);
              }}
            >
              Refresh cancelled data
            </Button>
          )}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        {state.status !== "cancelled" && (
          <Button type="submit" disabled={!confirmable || !confirmed || actionPending}>
            Confirm cancellation
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            if (leaveGuard.canLeave()) scope.onCancel();
          }}
        >
          {state.status === "cancelled" ? "Close cancellation" : "Keep task"}
        </Button>
      </div>
    </form>
  );
}
