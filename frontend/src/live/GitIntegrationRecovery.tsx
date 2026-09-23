import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import {
  gitRecoveryAttempt,
  type GitRecoveryAction,
  type GitRecoveryAttempt,
} from "../contracts/git-recovery-command.ts";
import type { GitIntegrations } from "../contracts/surface-artifact.ts";
import { LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendGitRecoveryCommand } from "./git-recovery-command-api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";

type Integration = GitIntegrations["items"][number];
type State =
  | { kind: "editing" | "preparing" }
  | { kind: "confirm" | "sending" | "unknown"; attempt: GitRecoveryAttempt }
  | { kind: "refused"; error: string }
  | { kind: "accepted"; revision: number; readbackFailed: boolean };

export function GitIntegrationRecovery({
  scope,
  taskId,
  cursor,
  integration,
}: {
  scope: ProjectReadScope;
  taskId: string;
  cursor: string | null;
  integration: Integration;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [reason, setReason] = useState("");
  const [state, setState] = useState<State>({ kind: "editing" });
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const pending =
    state.kind === "preparing" || state.kind === "sending" || state.kind === "unknown";
  const dirty = state.kind !== "accepted" && (reason.length > 0 || pending);
  const validReason =
    reason.trim().length > 0 &&
    !reason.includes("\0") &&
    new TextEncoder().encode(reason).byteLength <= 4096;

  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Git integration recovery? The reason and exact retry key will be lost; Core may have accepted an in-flight command.",
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
  }, [leaveGuard, dirty]);

  async function prepare(action: GitRecoveryAction) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "preparing" });
    try {
      const [project, task, page] = await session.request(generation, (token) =>
        Promise.all([
          api.project(projectId, token, controller.signal),
          api.task(projectId, taskId, token, controller.signal),
          api.taskSurface(projectId, taskId, "integrations", cursor, token, controller.signal),
        ]),
      );
      if (controller.signal.aborted || session.getSnapshot().generation !== generation) return;
      if (page.kind !== "integrations") throw Error("unexpected surface");
      const fresh = page.value.items.find((item) => item.id === integration.id);
      if (
        !fresh ||
        fresh.state !== "held" ||
        task.lifecycle !== "in_progress" ||
        (action === "accept_git_integration_result" && fresh.result_code !== "applied")
      ) {
        setState({
          kind: "refused",
          error: "This held operation or Task changed. Refresh its evidence.",
        });
        return;
      }
      setState({
        kind: "confirm",
        attempt: gitRecoveryAttempt(action, {
          project_id: projectId,
          expected_revision: project.revision,
          payload: {
            operation_id: fresh.id,
            expected_task_revision: task.revision,
            reason,
          },
        }),
      });
    } catch {
      if (!controller.signal.aborted)
        setState({
          kind: "refused",
          error: "Could not refresh Project, Task and integration. Try again.",
        });
    } finally {
      inFlight.current = false;
    }
  }

  async function submit(attempt: GitRecoveryAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendGitRecoveryCommand(
          fetch,
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (controller.signal.aborted || session.getSnapshot().generation !== generation) return;
      setState({ kind: "accepted", revision: receipt.project_revision, readbackFailed: false });
      try {
        const project = await session.request(generation, (token) =>
          api.project(projectId, token, controller.signal),
        );
        if (project.revision < receipt.project_revision) throw Error("readback behind receipt");
        await Promise.all([
          queries.invalidateQueries({ queryKey: readKeys.task(generation, projectId, taskId) }),
          queries.invalidateQueries({
            queryKey: ["live", generation, projectId, "task-surface", taskId],
          }),
        ]);
      } catch {
        if (!controller.signal.aborted)
          setState({ kind: "accepted", revision: receipt.project_revision, readbackFailed: true });
      }
    } catch (caught) {
      if (!controller.signal.aborted)
        setState(
          caught instanceof LiveCommandError && caught.kind !== "outcome_unknown"
            ? {
                kind: "refused",
                error: `Core refused recovery (${caught.kind}). Refresh the held operation and review it again.`,
              }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }

  if (integration.state !== "held") return null;
  return (
    <div className="mt-2 space-y-2 rounded border border-border p-2">
      <p className="text-xs text-muted-foreground">
        Recovery requires a current Task visit and Core gate check. Retry requests another attempt;
        accepting a recorded Applied result asks Core to reconcile that result again.
      </p>
      {state.kind !== "accepted" && (
        <label className="block text-sm">
          Recovery reason
          <textarea
            className="mt-1 min-h-16 w-full rounded border border-input bg-background p-2"
            value={reason}
            onChange={(event) => {
              setReason(event.target.value);
              setState({ kind: "editing" });
            }}
            disabled={pending}
          />
        </label>
      )}
      {reason.length > 0 && !validReason && (
        <p role="alert">Enter a nonblank reason of at most 4096 bytes without null characters.</p>
      )}
      {state.kind === "refused" && <p role="alert">{state.error}</p>}
      {state.kind === "preparing" && <p role="status">Refreshing current evidence…</p>}
      {state.kind === "confirm" && (
        <p role="status">
          Confirm{" "}
          {state.attempt.action === "retry_git_integration"
            ? "retry"
            : "acceptance of the recorded Applied result"}{" "}
          at Project revision {state.attempt.expectedRevision} and Task revision{" "}
          {state.attempt.expectedTaskRevision}. Core may refuse if its gates changed.
        </p>
      )}
      {state.kind === "unknown" && (
        <p role="alert">Outcome unknown. Retry this exact request and key to obtain the receipt.</p>
      )}
      {state.kind === "accepted" && (
        <p role="status">
          Core accepted the recovery request at Project revision {state.revision}. Reconciliation is
          pending.
          {state.readbackFailed ? " Project readback failed; refresh before another action." : ""}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {(state.kind === "editing" || state.kind === "refused") && (
          <>
            <Button
              variant="outline"
              disabled={!validReason}
              onClick={() => void prepare("retry_git_integration")}
            >
              Review retry
            </Button>
            {integration.result_code === "applied" && (
              <Button
                variant="outline"
                disabled={!validReason}
                onClick={() => void prepare("accept_git_integration_result")}
              >
                Review recorded Applied result
              </Button>
            )}
          </>
        )}
        {state.kind === "confirm" && (
          <Button onClick={() => void submit(state.attempt)}>Confirm recovery request</Button>
        )}
        {state.kind === "unknown" && (
          <Button onClick={() => void submit(state.attempt)}>Retry exact request</Button>
        )}
      </div>
    </div>
  );
}
