import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { managementAttempt, type ManagementAttempt } from "../contracts/management-command.ts";
import type { Escalation } from "../contracts/management.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";

type State =
  | { kind: "idle" }
  | { kind: "confirm" | "sending" | "unknown"; attempt: ManagementAttempt }
  | { kind: "accepted" | "refused"; message: string };

export function EscalationActions({
  scope,
  escalation,
}: {
  scope: ProjectReadScope;
  escalation: Escalation;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const [action, setAction] = useState<"submit_human_resolution" | "reroute_escalation">(
    "submit_human_resolution",
  );
  const [disposition, setDisposition] = useState<
    "continue_stage" | "needs_management_change" | "forward_to_human"
  >("needs_management_change");
  const [summary, setSummary] = useState("");
  const [outcome, setOutcome] = useState("");
  const [reason, setReason] = useState("");
  const [state, setState] = useState<State>({ kind: "idle" });
  const confirm = useRef<HTMLInputElement | null>(null);
  const inFlight = useRef(false);
  const abort = useRef<AbortController | null>(null);
  const key = readKeys.escalationDetail(generation, projectId, escalation.id);
  const detail = useQuery({
    queryKey: key,
    enabled: open,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.escalationDetail(projectId, escalation.id, token, signal),
      ),
    retry: false,
  });
  useEffect(() => () => abort.current?.abort(), []);
  useEffect(() => {
    if (state.kind !== "confirm" && state.kind !== "sending" && state.kind !== "unknown") return;
    return leaveGuard.register(
      () => true,
      "Leave prepared escalation action? Its replay key will be lost; an in-flight command may already be applied.",
    );
  }, [leaveGuard, state.kind]);

  async function prepare() {
    const controller = new AbortController();
    abort.current = controller;
    try {
      const [fresh, project] = await Promise.all([
        session.request(generation, (token) =>
          api.escalationDetail(projectId, escalation.id, token, controller.signal),
        ),
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
      ]);
      if (fresh.state.status !== "assigned" && fresh.state.status !== "queued")
        throw Error("Escalation changed");
      let payload: unknown;
      if (action === "submit_human_resolution") {
        const assignment = fresh.latest_assignment;
        if (
          fresh.state.status !== "assigned" ||
          !assignment ||
          assignment.resolver.kind !== "human" ||
          assignment.state.status !== "active" ||
          assignment.generation !== fresh.generation
        )
          throw Error("No current Human assignment");
        if (outcome && !fresh.allowed_outcomes.includes(outcome)) throw Error("Outcome changed");
        payload = {
          escalation_id: fresh.id,
          expected_escalation_revision: fresh.revision,
          assignment_id: assignment.id,
          lease_generation: assignment.generation,
          answer: {
            disposition,
            summary: summary.trim(),
            recommended_outcome_key: outcome || null,
          },
        };
      } else {
        payload = {
          escalation_id: fresh.id,
          expected_escalation_revision: fresh.revision,
          reason: reason.trim(),
        };
      }
      const attempt = managementAttempt(
        action,
        { project_id: projectId, expected_revision: project.revision, payload },
        crypto.randomUUID(),
      );
      setState({ kind: "confirm", attempt });
    } catch {
      setState({
        kind: "refused",
        message:
          "Current assignment, revision or answer is unavailable. Refresh this escalation before preparing again.",
      });
    } finally {
      abort.current = null;
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
      let readback = "Canonical detail unavailable; refresh before another action.";
      try {
        const fresh = await session.request(generation, (token) =>
          api.escalationDetail(projectId, escalation.id, token, controller.signal),
        );
        readback = `Current escalation: ${fresh.state.status}, revision ${fresh.revision}, generation ${fresh.generation}.`;
      } catch {
        /* receipt remains durable, readback is separate */
      }
      setState({
        kind: "accepted",
        message: `Core saved ${attempt.action} (${receipt.status}). ${readback}`,
      });
      void client.invalidateQueries({ queryKey: ["live", generation, projectId, "management"] });
      void client.invalidateQueries({ queryKey: key });
    } catch (error) {
      if (!(error instanceof LiveCommandError) || error.kind === "outcome_unknown")
        setState({ kind: "unknown", attempt });
      else
        setState({
          kind: "refused",
          message: `Core refused action: ${error.kind}. Refresh escalation and Project before a new attempt.`,
        });
    } finally {
      inFlight.current = false;
      abort.current = null;
    }
  }

  return (
    <div className="mt-2 space-y-2 border-t border-border pt-2">
      <Button variant="outline" onClick={() => setOpen((value) => !value)}>
        {open ? "Hide escalation detail" : "Open escalation detail"}
      </Button>
      {open && detail.isPending && <p role="status">Loading current escalation…</p>}
      {open && detail.isError && <p role="alert">{describeApiError(detail.error)}</p>}
      {open && detail.data && (
        <div className="space-y-2">
          <p>
            Current revision {detail.data.revision}, generation {detail.data.generation}. Current
            assignment: {detail.data.latest_assignment?.resolver.kind ?? "none"},{" "}
            {detail.data.latest_assignment?.state.status ?? "none"}.
          </p>
          {(state.kind === "idle" || state.kind === "refused") && (
            <>
              <label className="block">
                Action{" "}
                <select
                  className="w-full rounded border bg-background p-2"
                  value={action}
                  onChange={(event) => setAction(event.target.value as typeof action)}
                >
                  <option value="submit_human_resolution">Submit Human resolution</option>
                  <option value="reroute_escalation">Reroute to Human</option>
                </select>
              </label>
              {action === "submit_human_resolution" ? (
                <>
                  <label className="block">
                    Disposition{" "}
                    <select
                      className="w-full rounded border bg-background p-2"
                      value={disposition}
                      onChange={(event) => setDisposition(event.target.value as typeof disposition)}
                    >
                      <option value="continue_stage">Continue stage</option>
                      <option value="needs_management_change">Needs management change</option>
                      <option value="forward_to_human">Forward to Human</option>
                    </select>
                  </label>
                  <label className="block">
                    Summary{" "}
                    <textarea
                      className="w-full rounded border bg-background p-2"
                      value={summary}
                      onChange={(event) => setSummary(event.target.value)}
                      maxLength={20000}
                    />
                  </label>
                  <label className="block">
                    Recommended outcome (optional){" "}
                    <select
                      className="w-full rounded border bg-background p-2"
                      value={outcome}
                      onChange={(event) => setOutcome(event.target.value)}
                    >
                      <option value="">None</option>
                      {detail.data.allowed_outcomes.map((value) => (
                        <option key={value} value={value}>
                          {value}
                        </option>
                      ))}
                    </select>
                  </label>
                  <p className="text-xs text-muted-foreground">
                    A recommended outcome does not change Pipeline stage. Continue is accepted by
                    Core only after source execution is physically quiescent.
                  </p>
                </>
              ) : (
                <label className="block">
                  Reroute reason{" "}
                  <Input
                    value={reason}
                    onChange={(event) => setReason(event.target.value)}
                    maxLength={10000}
                  />
                </label>
              )}
              <Button variant="outline" onClick={() => void prepare()}>
                Prepare action from fresh detail
              </Button>
            </>
          )}
          {state.kind === "confirm" && (
            <div className="space-y-2 rounded border p-2">
              <p>
                Confirm exact {state.attempt.action} for escalation {state.attempt.resourceId} at
                Project revision {state.attempt.expectedRevision}.
              </p>
              <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] text-xs">
                {state.attempt.body}
              </pre>
              <label className="flex gap-2">
                <input ref={confirm} type="checkbox" />I confirm this exact action
              </label>
              <Button
                onClick={() => {
                  if (confirm.current?.checked) void send(state.attempt);
                }}
              >
                Submit
              </Button>
              <Button variant="outline" onClick={() => setState({ kind: "idle" })}>
                Cancel
              </Button>
            </div>
          )}
          {state.kind === "sending" && <p role="status">Saving escalation action…</p>}
          {state.kind === "unknown" && (
            <Button variant="outline" onClick={() => void send(state.attempt)}>
              Retry exact request and key
            </Button>
          )}
          {(state.kind === "accepted" || state.kind === "refused") && (
            <p role={state.kind === "accepted" ? "status" : "alert"}>{state.message}</p>
          )}
        </div>
      )}
    </div>
  );
}
