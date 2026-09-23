import { useEffect, useRef, useState } from "react";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  inboxAttempt,
  type InboxAttempt,
  type InboxReceipt,
} from "../contracts/communication-command.ts";
import type { MessageDelivery } from "../contracts/communication.ts";
import { LiveCommandError } from "./command-error.ts";
import type { ProjectReadScope } from "./read-scope.ts";

export function CommunicationRetry({
  scope,
  threadId,
  delivery,
  onApplied,
}: {
  scope: ProjectReadScope;
  threadId: string;
  delivery: MessageDelivery;
  onApplied: () => void;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const [reason, setReason] = useState("");
  const [attempt, setAttempt] = useState<InboxAttempt | null>(null);
  const [status, setStatus] = useState<
    "idle" | "confirm" | "sending" | "unknown" | "accepted" | "refused"
  >("idle");
  const [notice, setNotice] = useState("");
  const confirm = useRef<HTMLInputElement | null>(null);
  const abort = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => () => abort.current?.abort(), []);
  useEffect(() => {
    if (status !== "confirm" && status !== "sending" && status !== "unknown") return;
    return leaveGuard.register(
      () => true,
      "Leave Communication retry? The exact retry key will be lost; Core may have applied an in-flight request.",
    );
  }, [leaveGuard, status]);
  if ((!delivery.assignment?.retry_ready || !delivery.assignment.run_id) && status === "idle")
    return null;
  async function prepare() {
    const controller = new AbortController();
    abort.current = controller;
    try {
      // Recheck the exact message and physical readiness immediately before confirmation.
      const [page, project] = await Promise.all([
        session.request(generation, (token) =>
          api.messageDelivery(
            projectId,
            threadId,
            String(delivery.sequence - 1),
            token,
            controller.signal,
          ),
        ),
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
      ]);
      const fresh = page.items.find((item) => item.message_id === delivery.message_id);
      if (
        !fresh?.assignment?.retry_ready ||
        fresh.assignment.run_id !== delivery.assignment?.run_id
      )
        throw Error("Readiness changed");
      const prepared = inboxAttempt(
        "retry_communication",
        {
          project_id: projectId,
          expected_revision: project.revision,
          payload: { run_id: fresh.assignment.run_id, reason },
        },
        crypto.randomUUID(),
      );
      setAttempt(prepared);
      setStatus("confirm");
      setNotice("");
    } catch {
      setStatus("refused");
      setNotice("Current retry readiness or reason is unavailable. Refresh delivery evidence.");
    } finally {
      abort.current = null;
    }
  }
  async function send(frozen: InboxAttempt) {
    if (inFlight.current) return;
    inFlight.current = true;
    const controller = new AbortController();
    abort.current = controller;
    setStatus("sending");
    try {
      const receipt: InboxReceipt = await session.request(generation, (token) =>
        api.inboxCommand(frozen, token, controller.signal),
      );
      setStatus("accepted");
      setNotice(
        `Core saved retry request (${receipt.status}). A new delivery or answer is not yet proven.`,
      );
      onApplied();
    } catch (error) {
      if (!(error instanceof LiveCommandError) || error.kind === "outcome_unknown") {
        setStatus("unknown");
        setNotice("Outcome unknown. Retry the exact same request and key.");
      } else {
        setStatus("refused");
        setNotice(`Core refused retry: ${error.kind}. Refresh evidence before a new request.`);
      }
    } finally {
      inFlight.current = false;
      abort.current = null;
    }
  }
  return (
    <div className="space-y-2 border-t border-border pt-2 text-sm">
      <p>
        Core currently observes this held Communication attempt as eligible for retry. Readiness can
        change; Core checks it again at command time.
      </p>
      {(status === "idle" || status === "refused") && (
        <label className="block">
          Retry reason
          <Input
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            maxLength={4096}
          />
        </label>
      )}
      {(status === "idle" || status === "refused") && (
        <Button variant="outline" onClick={() => void prepare()}>
          Prepare Communication retry
        </Button>
      )}
      {status === "confirm" && attempt && (
        <div className="space-y-2 rounded border p-2">
          <p>
            Confirm retry for Run {attempt.resourceId}. Project revision {attempt.expectedRevision}.
            Physical readiness was just observed and may change.
          </p>
          <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] text-xs">{attempt.body}</pre>
          <label className="flex gap-2">
            <input ref={confirm} type="checkbox" />I confirm this exact retry
          </label>
          <Button
            onClick={() => {
              if (confirm.current?.checked) void send(attempt);
            }}
          >
            Request retry
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              setAttempt(null);
              setStatus("idle");
            }}
          >
            Cancel
          </Button>
        </div>
      )}
      {status === "sending" && <p role="status">Saving retry request…</p>}
      {status === "unknown" && attempt && (
        <Button variant="outline" onClick={() => void send(attempt)}>
          Retry exact request
        </Button>
      )}
      {notice && <p role={status === "accepted" ? "status" : "alert"}>{notice}</p>}
    </div>
  );
}
