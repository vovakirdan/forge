import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { managementAttempt, type ManagementAttempt } from "../contracts/management-command.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";

type State =
  | { kind: "idle" }
  | { kind: "confirm" | "sending" | "unknown"; attempt: ManagementAttempt; observed: string }
  | { kind: "accepted" | "refused"; message: string };

export function RecoveryAssessmentAction({
  scope,
  runId,
}: {
  scope: ProjectReadScope;
  runId: string;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<State>({ kind: "idle" });
  const key = readKeys.recoveryAssessmentReadiness(generation, projectId, runId);
  const readiness = useQuery({
    queryKey: key,
    enabled: open,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.recoveryAssessmentReadiness(projectId, runId, token, signal),
      ),
    retry: false,
  });
  const confirm = useRef<HTMLInputElement | null>(null);
  const abort = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => () => abort.current?.abort(), []);
  useEffect(() => {
    if (state.kind !== "confirm" && state.kind !== "sending" && state.kind !== "unknown") return;
    return leaveGuard.register(
      () => true,
      "Leave prepared recovery assessment? Its exact replay key will be lost; Core may already have applied the request.",
    );
  }, [leaveGuard, state.kind]);
  async function prepare() {
    const controller = new AbortController();
    abort.current = controller;
    try {
      const fresh = await session.request(generation, (token) =>
        api.recoveryAssessmentReadiness(projectId, runId, token, controller.signal),
      );
      if (!fresh.eligible || fresh.reason_code !== null || fresh.task_id === null)
        throw Error("not eligible");
      const attempt = managementAttempt(
        "accept_run_recovery_assessment",
        {
          project_id: projectId,
          expected_revision: fresh.project_revision,
          payload: { run_id: fresh.run_id, assessment: "not_started_confirmed" },
        },
        crypto.randomUUID(),
      );
      const observed = `Run revision ${fresh.run_revision}, lease fence ${fresh.lease_fencing_token}, environment epoch ${fresh.environment_epoch}, Task ${fresh.task_id} revision ${fresh.task_revision}.`;
      setState({ kind: "confirm", attempt, observed });
    } catch {
      setState({
        kind: "refused",
        message:
          "Current recovery readiness is unavailable or changed. Refresh the Run before preparing again.",
      });
    } finally {
      abort.current = null;
    }
  }
  async function send(frozen: Extract<State, { attempt: ManagementAttempt }>) {
    if (inFlight.current) return;
    inFlight.current = true;
    const controller = new AbortController();
    abort.current = controller;
    setState({ ...frozen, kind: "sending" });
    try {
      const receipt = await session.request(generation, (token) =>
        api.managementCommand(frozen.attempt, token, controller.signal),
      );
      let readback = "Canonical readiness readback unavailable. Refresh before another action.";
      try {
        const current = await session.request(generation, (token) =>
          api.recoveryAssessmentReadiness(projectId, runId, token, controller.signal),
        );
        readback = `Current eligibility: ${current.eligible ? "yes" : "no"}; reason ${current.reason_code ?? "none"}.`;
      } catch {
        /* accepted receipt remains distinct from readback */
      }
      setState({
        kind: "accepted",
        message: `Core saved assessment (${receipt.status}). ${readback} A new Run or Task outcome is not implied.`,
      });
      void client.invalidateQueries({
        queryKey: ["live", generation, projectId, "management-facts", "recovery-runs"],
      });
      void client.invalidateQueries({ queryKey: key });
    } catch (error) {
      if (!(error instanceof LiveCommandError) || error.kind === "outcome_unknown")
        setState({ ...frozen, kind: "unknown" });
      else
        setState({
          kind: "refused",
          message: `Core refused assessment: ${error.kind}. Refresh before a new attempt.`,
        });
    } finally {
      inFlight.current = false;
      abort.current = null;
    }
  }
  return (
    <div className="mt-2 space-y-2 border-t border-border pt-2">
      <Button variant="outline" onClick={() => setOpen((value) => !value)}>
        {open ? "Hide recovery readiness" : "Check assessment readiness"}
      </Button>
      {open && readiness.isPending && <p role="status">Checking canonical recovery facts…</p>}
      {open && readiness.isError && <p role="alert">{describeApiError(readiness.error)}</p>}
      {open && readiness.data && (
        <>
          <p>
            Core structural readiness for <code>not_started_confirmed</code>:{" "}
            {readiness.data.eligible ? "eligible" : "ineligible"}.{" "}
            {readiness.data.reason_code ? `Reason: ${readiness.data.reason_code}.` : ""}
          </p>
          <p className="text-xs text-muted-foreground">
            Only accept this assessment if you have positive evidence that no work began. Core
            checks structural facts again at command time. This does not prove an external effect by
            itself.
          </p>
          {(state.kind === "idle" || state.kind === "refused") && readiness.data.eligible && (
            <Button variant="outline" onClick={() => void prepare()}>
              Prepare no-work assessment
            </Button>
          )}
        </>
      )}
      {state.kind === "confirm" && (
        <div className="space-y-2 rounded border p-2">
          <p>
            Confirm assessment for Run {runId}. {state.observed}
          </p>
          <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] text-xs">
            {state.attempt.body}
          </pre>
          <label className="flex gap-2">
            <input ref={confirm} type="checkbox" />I have positive evidence no work began and
            confirm this exact assessment
          </label>
          <Button
            onClick={() => {
              if (confirm.current?.checked) void send(state);
            }}
          >
            Accept assessment
          </Button>
          <Button variant="outline" onClick={() => setState({ kind: "idle" })}>
            Cancel
          </Button>
        </div>
      )}
      {state.kind === "sending" && <p role="status">Saving recovery assessment…</p>}
      {state.kind === "unknown" && (
        <Button variant="outline" onClick={() => void send(state)}>
          Retry exact request and key
        </Button>
      )}
      {(state.kind === "accepted" || state.kind === "refused") && (
        <p role={state.kind === "accepted" ? "status" : "alert"}>{state.message}</p>
      )}
    </div>
  );
}
