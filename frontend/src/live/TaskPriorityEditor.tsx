import { startTransition, useActionState, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { TaskCommandAttempt, TaskCommandReceipt } from "../contracts/task-command.ts";
import { describeApiError } from "./api.ts";
import { describeCommandError, LiveCommandError } from "./command-error.ts";
import {
  activePriority,
  canChangePriority,
  createPriorityAttempt,
  type PriorityBaseline,
} from "./priority-attempt.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type EditorProps = ProjectReadScope & { taskId: string; onCancel: () => void };
async function readBaseline(scope: EditorProps, signal: AbortSignal): Promise<PriorityBaseline> {
  const { session, generation, api, projectId, taskId } = scope;
  const [scheme, task] = await Promise.all([
    session.request(generation, (token) => api.priorityScheme(projectId, token, signal)),
    session.request(generation, (token) => api.task(projectId, taskId, token, signal)),
  ]);
  return { scheme, task };
}

export function TaskPriorityEditor(scope: EditorProps) {
  const key = useMemo(
    () => ["live", scope.generation, scope.projectId, "priority-baseline", scope.taskId] as const,
    [scope.generation, scope.projectId, scope.taskId],
  );
  const baseline = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => readBaseline(scope, signal),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section aria-label="Change priority" className="space-y-3 rounded-lg border border-border p-4">
      <h3 className="font-semibold">Change priority</h3>
      {baseline.isPending && <p role="status">Loading current priority baseline…</p>}
      {baseline.isError && (
        <>
          <p role="alert">
            Cannot load the priority baseline. {describeApiError(baseline.error, "Task")}
          </p>
          <Button onClick={() => void baseline.refetch()} disabled={baseline.isFetching}>
            Retry priority baseline
          </Button>
        </>
      )}
      {baseline.data ? (
        <PriorityForm {...scope} initial={baseline.data} />
      ) : (
        <Button variant="outline" onClick={scope.onCancel}>
          Cancel
        </Button>
      )}
    </section>
  );
}

type SaveState =
  | { status: "editing" }
  | { status: "sending" | "unknown"; attempt: TaskCommandAttempt }
  | { status: "refused"; error: LiveCommandError }
  | { status: "saved"; receipt: TaskCommandReceipt; refreshing: boolean; refreshFailed: boolean };

function PriorityForm(scope: EditorProps & { initial: PriorityBaseline }) {
  const { api, session, generation, projectId, taskId, leaveGuard, onCancel } = scope;
  const queries = useQueryClient();
  const [baseline, setBaseline] = useState(scope.initial);
  const [selected, setSelected] = useState(baseline.task.priority);
  const [state, setState] = useState<SaveState>({ status: "editing" });
  const [refreshing, setRefreshing] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  const changed = selected !== baseline.task.priority;
  const warn =
    state.status === "sending" ||
    state.status === "unknown" ||
    (state.status !== "saved" && changed);
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
  async function refreshSaved(receipt: TaskCommandReceipt, controller: AbortController) {
    setState({ status: "saved", receipt, refreshing: true, refreshFailed: false });
    try {
      const [current, project] = await Promise.all([
        readBaseline(scope, controller.signal),
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
      ]);
      if (!stillCurrent(controller)) return;
      const projectKey = ["project", generation, projectId] as const;
      const taskKey = readKeys.task(generation, projectId, taskId);
      const schemeKey = readKeys.priorityScheme(generation, projectId);
      await Promise.all(
        [projectKey, taskKey, schemeKey].map((queryKey) =>
          queries.cancelQueries({ queryKey, exact: true }),
        ),
      );
      if (!stillCurrent(controller)) return;
      queries.setQueryData(projectKey, project);
      queries.setQueryData(taskKey, current.task);
      queries.setQueryData(schemeKey, current.scheme);
      setBaseline(current);
      setSelected(current.task.priority);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "tasks"] },
        { throwOnError: true },
      );
      if (stillCurrent(controller))
        setState({ status: "saved", receipt, refreshing: false, refreshFailed: false });
    } catch {
      if (stillCurrent(controller))
        setState({ status: "saved", receipt, refreshing: false, refreshFailed: true });
    }
  }
  async function submit(attempt: TaskCommandAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.setTaskPriority(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (stillCurrent(controller)) await refreshSaved(receipt, controller);
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
    try {
      const current = await readBaseline(scope, controller.signal);
      if (!stillCurrent(controller)) return;
      // Keep the selected intent even if the new catalog no longer permits it.
      setBaseline(current);
      setState({ status: "editing" });
    } catch (error) {
      if (stillCurrent(controller)) setReadError(describeApiError(error, "Task"));
    } finally {
      if (stillCurrent(controller)) setRefreshing(false);
    }
  }
  const valid = activePriority(baseline.scheme, selected);
  const revisionSupported =
    baseline.scheme.project_revision < Number.MAX_SAFE_INTEGER &&
    baseline.task.revision < Number.MAX_SAFE_INTEGER;
  const editable =
    state.status === "editing" && canChangePriority(baseline.task.lifecycle) && revisionSupported;
  const canSave = editable && changed && valid;
  const [, save, actionPending] = useActionState(async (_previous: null) => {
    if (canSave) await submit(createPriorityAttempt(baseline, selected));
    return null;
  }, null);
  const currentLevel = baseline.scheme.levels.find((level) => level.id === baseline.task.priority);
  return (
    <form
      // A host form action resets native selects during commit, even on a handled
      // refusal. Dispatch explicitly so recovery retains the controlled selection.
      onSubmit={(event) => {
        event.preventDefault();
        startTransition(save);
      }}
      className="space-y-3"
    >
      <p className="text-xs text-muted-foreground">
        Only priority changes. Lifecycle and Pipeline stage stay unchanged; a current Run is not
        interrupted.
      </p>
      <p className="text-xs">
        Priority baseline: Project revision {baseline.scheme.project_revision}, Task revision{" "}
        {baseline.task.revision}.
      </p>
      <p className="text-sm [overflow-wrap:anywhere]">
        {state.status === "sending" ||
        state.status === "unknown" ||
        state.status === "refused" ||
        (state.status === "saved" && (state.refreshing || state.refreshFailed))
          ? "Last loaded priority"
          : "Current saved priority"}
        : {currentLevel?.display_name ?? "Name unavailable"} ({baseline.task.priority})
        {currentLevel?.retired ? " (retired)" : ""}.
      </p>
      {!canChangePriority(baseline.task.lifecycle) && (
        <p role="alert">
          This Task is closed. Priority editing is unavailable; your selection is retained.
        </p>
      )}
      {!revisionSupported && (
        <p role="alert">
          The revision exceeds the browser edit limit. This priority cannot be saved here.
        </p>
      )}
      <div className="space-y-2">
        <label htmlFor="task-priority">Task priority</label>
        <select
          id="task-priority"
          className="block w-full min-w-0 rounded border border-border bg-background p-2"
          value={selected}
          disabled={!editable || actionPending}
          onChange={(event) => setSelected(event.target.value)}
          aria-invalid={!valid}
          aria-describedby="priority-help"
        >
          {!valid && (
            <option value={selected} disabled>
              {selected} (unavailable; choose an active priority)
            </option>
          )}
          {baseline.scheme.levels
            .filter((level) => !level.retired)
            .map((level) => (
              <option key={level.id} value={level.id}>
                {level.display_name} ({level.id})
              </option>
            ))}
        </select>
        <p id="priority-help" className="text-xs">
          {valid
            ? "Select an active priority from the Project catalog."
            : "Your selected priority is unavailable or retired. Choose an active priority before saving."}
        </p>
      </div>
      {state.status === "sending" && <p role="status">Saving priority…</p>}
      {state.status === "unknown" && (
        <>
          <p role="alert">{describeCommandError(new LiveCommandError("outcome_unknown"))}</p>
          <Button type="button" disabled={actionPending} onClick={() => void submit(state.attempt)}>
            Retry same save
          </Button>
        </>
      )}
      {state.status === "refused" && (
        <>
          <p role="alert">{describeCommandError(state.error)}</p>
          <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
            Refresh priority baseline
          </Button>
        </>
      )}
      {readError && <p role="alert">{readError} Your selection has been retained.</p>}
      {state.status === "saved" && (
        <>
          <p role="status">
            Saved.{" "}
            {state.refreshing
              ? "Refreshing authoritative data…"
              : state.refreshFailed
                ? "Could not refresh authoritative data. The save is confirmed."
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
                if (controller) void refreshSaved(state.receipt, controller);
              }}
            >
              Refresh saved data
            </Button>
          )}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        {state.status !== "saved" && (
          <Button type="submit" disabled={!canSave || actionPending}>
            Save priority
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            if (leaveGuard.canLeave()) onCancel();
          }}
        >
          {state.status === "saved" ? "Close editor" : "Cancel"}
        </Button>
      </div>
    </form>
  );
}
