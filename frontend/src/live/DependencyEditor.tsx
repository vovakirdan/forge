import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import {
  dependencyAttempt,
  type DependencyCommandAttempt,
  type DependencyCommandReceipt,
} from "../contracts/dependency-command.ts";
import type { DependencyDirection } from "../contracts/task-dependencies.ts";
import type { TaskSummaryView } from "../contracts/task.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { describeCommandError, LiveCommandError } from "./command-error.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { invalidateProjectDependencies, readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Props = ProjectReadScope & {
  taskId: string;
  taskKey: string;
  direction: DependencyDirection;
  action: "create" | "remove";
  related?: { id: string; key: string; title: string };
  onClose: () => void;
  onAccepted: () => void;
};
type Result =
  | { state: "confirm" }
  | { state: "sending" | "unknown"; attempt: DependencyCommandAttempt }
  | { state: "refused"; error: LiveCommandError }
  | { state: "accepted"; receipt: DependencyCommandReceipt };

export function DependencyEditor(scope: Props) {
  const { api, session, generation, projectId, taskId, direction, action, leaveGuard } = scope;
  const queries = useQueryClient();
  const [selected, setSelected] = useState<TaskSummaryView | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [result, setResult] = useState<Result>({ state: "confirm" });
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState(false);
  const inFlight = useRef(false);
  const lifetime = useRef<AbortController | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  const projectKey = useMemo(
    () => ["project", generation, projectId, "dependency-edit"] as const,
    [generation, projectId],
  );
  const project = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(projectKey);
  const pickerKey = useMemo(
    () => ["live", generation, projectId, "dependency-picker", cursor] as const,
    [generation, projectId, cursor],
  );
  const picker = useQuery({
    queryKey: pickerKey,
    enabled: action === "create",
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.tasks(projectId, cursor, token, signal)),
    retry: false,
  });
  useReadLifetime(pickerKey);
  const related = action === "remove" ? scope.related : selected;
  const blockerId = direction === "blocked_by" ? related?.id : taskId;
  const blockedId = direction === "blocked_by" ? taskId : related?.id;
  const blockerKey = direction === "blocked_by" ? related?.key : scope.taskKey;
  const blockedKey = direction === "blocked_by" ? scope.taskKey : related?.key;
  const canConfirm =
    result.state === "confirm" &&
    project.isSuccess &&
    !project.isFetching &&
    blockerId !== undefined &&
    blockedId !== undefined &&
    project.data.revision < Number.MAX_SAFE_INTEGER;
  const warn =
    result.state === "sending" ||
    result.state === "unknown" ||
    (result.state !== "accepted" && selected !== null);
  useEffect(() => {
    if (!warn) return;
    const unregister = leaveGuard.register(() => true);
    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [leaveGuard, warn]);
  function current(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function send(attempt: DependencyCommandAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setResult({ state: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.dependencyCommand(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (!current(controller)) return;
      setResult({ state: "accepted", receipt });
      scope.onAccepted();
      void invalidateProjectDependencies(queries, generation, projectId);
      void queries.invalidateQueries({ queryKey: ["project", generation, projectId] });
      void queries.invalidateQueries({ queryKey: ["live", generation, projectId, "tasks"] });
      for (const id of [attempt.blockerId, attempt.blockedId])
        void queries.invalidateQueries({ queryKey: readKeys.task(generation, projectId, id) });
    } catch (error) {
      if (!current(controller)) return;
      setResult(
        error instanceof LiveCommandError && error.kind !== "outcome_unknown"
          ? { state: "refused", error }
          : { state: "unknown", attempt },
      );
    } finally {
      inFlight.current = false;
    }
  }
  async function refreshBaseline() {
    setRefreshing(true);
    setRefreshError(false);
    try {
      const response = await project.refetch();
      if (response.isError) throw response.error;
      setResult({ state: "confirm" });
    } catch {
      setRefreshError(true);
    } finally {
      setRefreshing(false);
    }
  }
  function navigate(next: string | null, trail: (string | null)[]) {
    setCursor(next);
    setPrevious(trail);
    setSelected(null);
  }
  return (
    <section
      aria-label="Edit dependency"
      className="space-y-3 rounded-lg border border-primary p-4"
    >
      <h3 className="font-semibold">
        {action === "create" ? "Add dependency" : "Remove dependency"}
      </h3>
      <p className="text-sm">
        Only task_done links are supported. Core decides waiting and execution state.
      </p>
      {action === "create" && (
        <>
          <p>Choose a Task in this Project:</p>
          {picker.isPending && <p role="status">Loading Tasks…</p>}
          {picker.isError && <p role="alert">{describeApiError(picker.error, "Task")}</p>}
          {picker.isError && <Button onClick={() => void picker.refetch()}>Retry Tasks</Button>}
          {picker.data && (
            <ul aria-label="Dependency candidates" className="space-y-2">
              {picker.data.items
                .filter((item) => item.id.toLowerCase() !== taskId.toLowerCase())
                .map((item) => (
                  <li key={item.id}>
                    <Button
                      variant={selected?.id === item.id ? "default" : "outline"}
                      disabled={result.state !== "confirm" || picker.isFetching}
                      onClick={() => setSelected(item)}
                    >
                      {item.key} — {item.title} ({item.lifecycle})
                    </Button>
                  </li>
                ))}
            </ul>
          )}
          {picker.data && (
            <nav aria-label="Candidate pages" className="flex items-center gap-3">
              <Button
                variant="outline"
                disabled={picker.isFetching || previous.length === 0}
                onClick={() => navigate(previous.at(-1) ?? null, previous.slice(0, -1))}
              >
                Previous
              </Button>
              <span>Page {previous.length + 1}</span>
              <Button
                variant="outline"
                disabled={picker.isFetching || picker.isError || !picker.data.next_cursor}
                onClick={() => {
                  if (picker.data?.next_cursor)
                    navigate(picker.data.next_cursor, [...previous, cursor]);
                }}
              >
                Next
              </Button>
            </nav>
          )}
          {picker.error instanceof LiveApiError && picker.error.kind === "cursor_invalid" && (
            <Button variant="outline" onClick={() => navigate(null, [])}>
              Restart candidate pages
            </Button>
          )}
        </>
      )}
      {related && (
        <p className="font-medium">
          {blockerKey} must finish before {blockedKey}.
        </p>
      )}
      {action === "remove" && (
        <p className="text-sm">
          Removing this link may release a wait. Core controls any later dispatch.
        </p>
      )}
      {project.isPending && <p role="status">Loading Project revision…</p>}
      {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
      {result.state === "refused" && <p role="alert">{describeCommandError(result.error)}</p>}
      {result.state === "unknown" && (
        <p role="alert">Outcome unknown. Retry the same command; do not submit a new one.</p>
      )}
      {result.state === "accepted" && (
        <p role="status">
          Core accepted the command ({result.receipt.status}). Dependency lists are refreshing.
        </p>
      )}
      {refreshError && <p role="alert">Could not refresh the Project. Retry before confirming.</p>}
      {result.state === "sending" && <p role="status">Sending command…</p>}
      {result.state === "confirm" && (
        <Button
          disabled={!canConfirm}
          onClick={() => {
            if (project.data && blockerId && blockedId)
              void send(
                dependencyAttempt(
                  action === "create" ? "create_dependency" : "remove_dependency",
                  projectId,
                  project.data.revision,
                  blockerId,
                  blockedId,
                ),
              );
          }}
        >
          Confirm {action === "create" ? "link" : "removal"}
        </Button>
      )}
      {result.state === "unknown" && (
        <Button onClick={() => void send(result.attempt)}>Retry same command</Button>
      )}
      {result.state === "refused" && (
        <Button disabled={refreshing} onClick={() => void refreshBaseline()}>
          Refresh baseline
        </Button>
      )}
      <Button
        variant="outline"
        onClick={() => {
          if (leaveGuard.canLeave()) scope.onClose();
        }}
      >
        Close
      </Button>
    </section>
  );
}
