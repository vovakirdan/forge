import { startTransition, useActionState, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { TaskCommandAttempt, TaskCommandReceipt } from "../contracts/task-command.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { describeApprovalError } from "./approval-error.ts";
import {
  createApprovalAttempt,
  hasApprovalDoD,
  type ApprovalBaseline,
} from "./approval-attempt.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type ApprovalProps = ProjectReadScope & {
  taskId: string;
  onCancel: () => void;
  onEdit: () => void;
};
async function readBaseline(scope: ApprovalProps, signal: AbortSignal): Promise<ApprovalBaseline> {
  const { session, generation, api, projectId, taskId } = scope;
  const [project, task] = await Promise.all([
    session.request(generation, (token) => api.project(projectId, token, signal)),
    session.request(generation, (token) => api.task(projectId, taskId, token, signal)),
  ]);
  const pipeline = await session.request(generation, (token) =>
    api.pipeline(projectId, task.pipeline_version_id, token, signal),
  );
  return { project, task, pipeline };
}

export function TaskApprovalPanel(scope: ApprovalProps) {
  const key = useMemo(
    () => ["live", scope.generation, scope.projectId, "approval-baseline", scope.taskId] as const,
    [scope.generation, scope.projectId, scope.taskId],
  );
  const baseline = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => readBaseline(scope, signal),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section aria-label="Approve draft" className="space-y-3 rounded-lg border border-border p-4">
      <h3 className="font-semibold">Approve draft</h3>
      {baseline.isPending && <p role="status">Loading current approval baseline…</p>}
      {baseline.isError && (
        <>
          <p role="alert">
            Cannot load the approval baseline. {describeApiError(baseline.error, "Task")}
          </p>
          <Button disabled={baseline.isFetching} onClick={() => void baseline.refetch()}>
            Retry approval baseline
          </Button>
        </>
      )}
      {baseline.data ? (
        <ApprovalForm {...scope} initial={baseline.data} />
      ) : (
        <Button variant="outline" onClick={scope.onCancel}>
          Cancel
        </Button>
      )}
    </section>
  );
}

type ApprovalState =
  | { status: "confirming" }
  | { status: "sending" | "unknown"; attempt: TaskCommandAttempt }
  | { status: "refused"; error: LiveCommandError }
  | {
      status: "approved";
      receipt: TaskCommandReceipt;
      refreshing: boolean;
      refreshFailed: boolean;
    };

function ApprovalForm(scope: ApprovalProps & { initial: ApprovalBaseline }) {
  const { api, session, generation, projectId, taskId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [baseline, setBaseline] = useState(scope.initial);
  const [confirmed, setConfirmed] = useState(false);
  const [state, setState] = useState<ApprovalState>({ status: "confirming" });
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
    (state.status !== "approved" && confirmed);
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
  async function refreshApproved(receipt: TaskCommandReceipt, controller: AbortController) {
    setState({ status: "approved", receipt, refreshing: true, refreshFailed: false });
    try {
      // Approval can already have started work. Do not infer a lifecycle or Task revision.
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
        setState({ status: "approved", receipt, refreshing: false, refreshFailed: false });
    } catch {
      if (stillCurrent(controller))
        setState({ status: "approved", receipt, refreshing: false, refreshFailed: true });
    }
  }
  async function submit(attempt: TaskCommandAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.approveTask(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (stillCurrent(controller)) await refreshApproved(receipt, controller);
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
  const hasDoD = hasApprovalDoD(baseline);
  const isDraft = baseline.task.lifecycle === "draft";
  const revisionSupported =
    baseline.project.revision < Number.MAX_SAFE_INTEGER &&
    baseline.task.revision < Number.MAX_SAFE_INTEGER;
  const confirmable = state.status === "confirming" && hasDoD && isDraft && revisionSupported;
  const [, approve, actionPending] = useActionState(async (_previous: null) => {
    if (confirmable && confirmed) await submit(createApprovalAttempt(baseline));
    return null;
  }, null);
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        startTransition(approve);
      }}
      className="space-y-3"
    >
      <p className="text-sm">
        Approval permits execution. If the Project gate is open, work may start immediately. This
        command does not open the execution gate.
      </p>
      <p className="text-xs">
        Core checks required properties, WorkSurface and Pipeline requirements; a saved DoD alone
        does not guarantee approval.
      </p>
      <dl className="space-y-2 text-sm [overflow-wrap:anywhere]">
        <dt>Saved title</dt>
        <dd className="whitespace-pre-wrap">{baseline.task.title}</dd>
        <dt>Saved definition of done</dt>
        <dd className="whitespace-pre-wrap">
          {baseline.task.definition_of_done ?? "Not provided."}
        </dd>
        <dt>Pinned Pipeline</dt>
        <dd>
          {baseline.pipeline.name} · version {baseline.pipeline.version} · {baseline.pipeline.id}
        </dd>
        <dt>Execution gate at confirmation baseline</dt>
        <dd>{baseline.project.execution_gate}</dd>
      </dl>
      <p className="text-xs">
        Approval baseline: Project revision {baseline.project.revision}, Task revision{" "}
        {baseline.task.revision}.
      </p>
      {!isDraft && <p role="alert">This Task is no longer a draft. Approval is unavailable.</p>}
      {!revisionSupported && (
        <p role="alert">
          The revision exceeds the browser command limit. Approval is unavailable here.
        </p>
      )}
      {!hasDoD && (
        <>
          <p role="alert">Save a nonblank definition of done before approval.</p>
          <Button
            type="button"
            disabled={state.status !== "confirming" || !isDraft}
            onClick={() => {
              if (leaveGuard.canLeave()) scope.onEdit();
            }}
          >
            Edit draft to add DoD
          </Button>
        </>
      )}
      {state.status !== "approved" && (
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={!confirmable || actionPending}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          I understand approval permits execution
        </label>
      )}
      {state.status === "sending" && <p role="status">Approving draft…</p>}
      {state.status === "unknown" && (
        <>
          <p role="alert">{describeApprovalError(new LiveCommandError("outcome_unknown"))}</p>
          <Button type="button" disabled={actionPending} onClick={() => void submit(state.attempt)}>
            Retry same approval
          </Button>
        </>
      )}
      {state.status === "refused" && (
        <>
          <p role="alert">{describeApprovalError(state.error)}</p>
          <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
            Refresh approval baseline
          </Button>
        </>
      )}
      {readError && <p role="alert">{readError}</p>}
      {state.status === "approved" && (
        <>
          <p role="status">
            Approved.{" "}
            {state.refreshing
              ? "Refreshing authoritative data…"
              : state.refreshFailed
                ? "Could not refresh authoritative data. Approval is confirmed."
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
                if (controller) void refreshApproved(state.receipt, controller);
              }}
            >
              Refresh approved data
            </Button>
          )}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        {state.status !== "approved" && (
          <Button type="submit" disabled={!confirmable || !confirmed || actionPending}>
            Confirm approval
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            if (leaveGuard.canLeave()) scope.onCancel();
          }}
        >
          {state.status === "approved" ? "Close approval" : "Cancel"}
        </Button>
      </div>
    </form>
  );
}
