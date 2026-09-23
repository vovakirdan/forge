import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import {
  systemJobAttempt,
  type SystemJobAction,
  type SystemJobAttempt,
} from "../contracts/system-job-command.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendSystemJobCommand } from "./system-job-command-api.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Props = ProjectReadScope & {
  action: SystemJobAction;
  label: string;
  identity?: string;
  payload: () => unknown;
  disabled?: boolean;
  onApplied: () => void;
};
type Status =
  | { kind: "editing" }
  | { kind: "sending" | "unknown"; attempt: SystemJobAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; attempt: SystemJobAttempt; resourceId: string | null; readError: boolean };

export function SystemJobCommandButton({
  action,
  label,
  identity,
  payload,
  disabled,
  onApplied,
  api,
  session,
  generation,
  projectId,
  leaveGuard,
}: Props) {
  const queries = useQueryClient();
  const [status, setStatus] = useState<Status>({ kind: "editing" });
  const [confirm, setConfirm] = useState(false);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const confirmRef = useRef<HTMLHeadingElement>(null);
  const baselineKey = useMemo(
    () =>
      [
        "live",
        generation,
        projectId,
        "system-job-command-baseline",
        action,
        identity ?? null,
      ] as const,
    [generation, projectId, action, identity],
  );
  const baseline = useQuery({
    queryKey: baselineKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(baselineKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    return () => next.abort();
  }, []);
  useEffect(() => {
    if (confirm) confirmRef.current?.focus();
  }, [confirm]);
  useEffect(() => {
    if (status.kind !== "sending" && status.kind !== "unknown") return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave this System Job action? The frozen retry key will be lost, and Core may already have applied the command.",
    );
    return unregister;
  }, [leaveGuard, status.kind]);
  function current(next: AbortController) {
    return !next.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function readAccepted(attempt: SystemJobAttempt, resourceId: string | null) {
    const next = controller.current;
    if (!next) return;
    setStatus({ kind: "accepted", attempt, resourceId, readError: false });
    try {
      const project = await session.request(generation, (token) =>
        api.project(projectId, token, next.signal),
      );
      if (project.revision < attempt.expectedProjectRevision + 1)
        throw new Error("Project readback absent");
      if (action === "skip_employee_onboarding") {
        const employee = attempt.expectedResourceId;
        if (!employee) throw new Error("Employee ID absent");
        const onboarding = await session.request(generation, (token) =>
          api.employeeOnboarding(projectId, employee, token, next.signal),
        );
        if (onboarding.state !== "skipped") throw new Error("Onboarding skip readback absent");
      } else {
        const jobs = await session.request(generation, (token) =>
          api.systemJobs(projectId, token, next.signal),
        );
        if (action === "configure_system_jobs") {
          const request = JSON.parse(attempt.body) as {
            payload: { expected_settings_revision: number };
          };
          if (jobs.settings_revision !== request.payload.expected_settings_revision + 1)
            throw new Error("Settings revision readback absent");
        } else if (!jobs.jobs.some((job) => job.id.toLowerCase() === resourceId?.toLowerCase()))
          throw new Error("System Job readback absent");
      }
      if (!current(next)) return;
      await queries.cancelQueries({ queryKey: ["project", generation, projectId], exact: true });
      queries.setQueryData(["project", generation, projectId], project);
      void queries.invalidateQueries({ queryKey: ["live", generation, projectId, "system-jobs"] });
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "employee-onboarding"],
      });
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "system-job-command-baseline"],
      });
      setStatus({ kind: "editing" });
      onApplied();
    } catch {
      if (current(next)) setStatus({ kind: "accepted", attempt, resourceId, readError: true });
    }
  }
  async function send(attempt: SystemJobAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setStatus({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendSystemJobCommand(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt, receipt.resource?.id ?? null);
    } catch (error) {
      if (current(next))
        setStatus(
          error instanceof LiveCommandError && error.kind !== "outcome_unknown"
            ? { kind: "refused", error }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  function prepare() {
    if (!baseline.data || baseline.isFetching) return;
    try {
      const attempt = systemJobAttempt(action, {
        project_id: projectId,
        expected_revision: baseline.data.revision,
        payload: payload(),
      });
      setFieldError(null);
      setConfirm(false);
      void send(attempt);
    } catch {
      setFieldError("Check this action's IDs, reason, policy, binding, and revisions.");
    }
  }
  return (
    <div className="space-y-2">
      {(status.kind === "editing" || status.kind === "refused") && !confirm && (
        <Button
          variant="outline"
          disabled={disabled || baseline.isPending || baseline.isFetching || baseline.isError}
          onClick={() => setConfirm(true)}
        >
          {label}
        </Button>
      )}
      {confirm && (
        <div
          role="alertdialog"
          aria-labelledby={`system-job-confirm-${action}`}
          className="space-y-2 rounded border border-border p-3"
        >
          <h4 id={`system-job-confirm-${action}`} ref={confirmRef} tabIndex={-1}>
            Confirm {label}
          </h4>
          <p className="text-sm">Core records an audited Project revision for this action.</p>
          <Button onClick={prepare}>Confirm</Button>
          <Button variant="outline" onClick={() => setConfirm(false)}>
            Cancel
          </Button>
        </div>
      )}
      {baseline.isError && <p role="alert">{describeApiError(baseline.error)}</p>}
      {fieldError && <p role="alert">{fieldError}</p>}
      {status.kind === "sending" && <p role="status">Sending System Job command…</p>}
      {status.kind === "unknown" && (
        <div className="space-y-2">
          <p role="alert">Outcome unknown. Retry the exact same request and key.</p>
          <Button onClick={() => void send(status.attempt)}>Retry same request</Button>
        </div>
      )}
      {status.kind === "refused" && (
        <div className="space-y-2">
          <p role="alert">
            Core refused this action ({status.error.kind}). Your fields remain. Refresh the Project
            before trying again.
          </p>
          <Button onClick={() => void baseline.refetch()}>Refresh Project</Button>
        </div>
      )}
      {status.kind === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Command accepted.{" "}
            {status.readError ? "Canonical readback failed." : "Reading canonical state…"}
          </p>
          {status.readError && (
            <Button onClick={() => void readAccepted(status.attempt, status.resourceId)}>
              Retry canonical read
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
