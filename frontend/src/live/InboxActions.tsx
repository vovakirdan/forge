import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  inboxAttempt,
  type InboxAction,
  type InboxAttempt,
  type InboxReceipt,
} from "../contracts/communication-command.ts";
import type {
  EmployeeMessage,
  EmployeeThread,
  MessageDelivery,
} from "../contracts/communication.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { LiveCommandError } from "./command-error.ts";

const kinds = ["instruction", "question", "notification"] as const;
const requirements = ["informational", "acknowledged", "answered"] as const;
type ActionState =
  | { status: "idle" }
  | { status: "sending" | "unknown"; attempt: InboxAttempt }
  | { status: "accepted"; receipt: InboxReceipt }
  | { status: "refused"; message: string };

function refusal(error: LiveCommandError): string {
  switch (error.kind) {
    case "stale_revision":
      return "Project or thread revision changed. Refresh before composing a new request.";
    case "conflict":
      return "The current conversation state conflicts with this request. Refresh before composing a new request.";
    case "idempotency_conflict":
      return "This retry key belongs to another request. Refresh before composing a new request.";
    case "validation_failed":
      return "Core refused the current target or requirement. Refresh and review the fields.";
    case "invalid_request":
      return "Core refused the request fields. Review them and refresh.";
    case "not_found":
      return "The Employee, Task, thread, Run or message is no longer in this Project.";
    case "forbidden":
      return "Core refused this management action.";
    case "outcome_unknown":
      return "Outcome unknown. Retry the exact same request to obtain a receipt, or refresh the conversation.";
  }
}

export function InboxActions({
  scope,
  employeeId,
  thread,
  message,
  delivery,
  onApplied,
}: {
  scope: ProjectReadScope;
  employeeId: string;
  thread: EmployeeThread | null;
  message?: EmployeeMessage;
  delivery?: MessageDelivery | undefined;
  onApplied: (receipt: InboxReceipt) => void;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const client = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [taskId, setTaskId] = useState("");
  const [body, setBody] = useState("");
  const [kind, setKind] = useState<(typeof kinds)[number]>("question");
  const [requirement, setRequirement] = useState<(typeof requirements)[number]>("answered");
  const [targetKind, setTargetKind] = useState<"inbox" | "task_execution" | "exact_run">("inbox");
  const [pipelineId, setPipelineId] = useState("");
  const [stageId, setStageId] = useState("");
  const [stageVisit, setStageVisit] = useState("");
  const [runId, setRunId] = useState("");
  const [fence, setFence] = useState("");
  const [epoch, setEpoch] = useState("");
  const [reason, setReason] = useState("");
  const [state, setState] = useState<ActionState>({ status: "idle" });
  const abort = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => () => abort.current?.abort(), []);
  useEffect(() => {
    if (!editing && state.status !== "sending" && state.status !== "unknown") return;
    return leaveGuard.register(
      () => true,
      "Leave Inbox editing? Unsaved text and the retry key will be lost. An in-flight command may still have been applied by Core.",
    );
  }, [editing, leaveGuard, state.status]);
  const action: InboxAction = message
    ? "waive_message_requirement"
    : thread
      ? "send_employee_message"
      : "open_employee_thread";

  async function submit(existing?: InboxAttempt) {
    if (inFlight.current) return;
    inFlight.current = true;
    const controller = new AbortController();
    abort.current = controller;
    let attempt = existing;
    try {
      if (!attempt) {
        const project = await session.request(generation, (token) =>
          api.project(projectId, token, controller.signal),
        );
        let payload: unknown;
        if (action === "open_employee_thread") {
          payload = { employee_id: employeeId, task_id: taskId.trim() || null };
        } else if (action === "waive_message_requirement") {
          payload = { message_id: message!.id, reason };
        } else {
          const context = {
            task_id: thread!.task_id ?? taskId.trim(),
            pipeline_version_id: pipelineId.trim(),
            stage_id: stageId.trim(),
            stage_visit: Number(stageVisit),
          };
          const target =
            targetKind === "inbox"
              ? { kind: "inbox" }
              : targetKind === "task_execution"
                ? { kind: "task_execution", context }
                : {
                    kind: "exact_run",
                    context,
                    run_id: runId.trim(),
                    fencing_token: Number(fence),
                    environment_epoch: Number(epoch),
                  };
          payload = {
            thread_id: thread!.id,
            expected_thread_revision: thread!.revision,
            target,
            kind,
            requirement: kind === "notification" ? "informational" : requirement,
            body,
            reply_to: null,
          };
        }
        attempt = inboxAttempt(action, {
          project_id: projectId,
          expected_revision: project.revision,
          payload,
        });
      }
      setState({ status: "sending", attempt });
      const receipt = await session.request(generation, (token) =>
        api.inboxCommand(attempt!, token, controller.signal),
      );
      setState({ status: "accepted", receipt });
      setEditing(false);
      onApplied(receipt);
      void client.invalidateQueries({
        queryKey: ["live", generation, projectId, "employee-threads"],
      });
      if (thread) {
        void client.invalidateQueries({
          queryKey: ["live", generation, projectId, "employee-messages", employeeId, thread.id],
        });
        void client.invalidateQueries({
          queryKey: ["live", generation, projectId, "message-delivery", thread.id],
        });
      }
    } catch (error) {
      if (attempt && (!(error instanceof LiveCommandError) || error.kind === "outcome_unknown"))
        setState({ status: "unknown", attempt });
      else
        setState({
          status: "refused",
          message:
            error instanceof LiveCommandError
              ? refusal(error)
              : "Check the fields and current Project revision.",
        });
    } finally {
      inFlight.current = false;
      abort.current = null;
    }
  }
  const canWaive =
    message &&
    message.requirement !== "informational" &&
    !delivery?.waiver &&
    !(message.requirement === "answered"
      ? delivery?.answered_at
      : delivery?.acknowledged_at || delivery?.answered_at);
  if (message && !canWaive) return null;
  return (
    <div className="space-y-2 text-sm">
      {!editing && state.status !== "unknown" && (
        <Button
          variant="outline"
          onClick={() => {
            setEditing(true);
            setState({ status: "idle" });
          }}
        >
          {message ? "Waive requirement" : thread ? "Compose message" : "Open thread"}
        </Button>
      )}
      {editing && state.status !== "accepted" && state.status !== "unknown" && (
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          {action === "open_employee_thread" && (
            <label className="block">
              Optional Task ID
              <Input
                value={taskId}
                onChange={(event) => setTaskId(event.target.value)}
                placeholder="Leave empty for a Taskless conversation"
              />
            </label>
          )}
          {action === "send_employee_message" && (
            <>
              <label className="block">
                Message kind{" "}
                <select
                  value={kind}
                  onChange={(event) => setKind(event.target.value as typeof kind)}
                  className="rounded border p-2"
                >
                  {kinds.map((value) => (
                    <option key={value} value={value}>
                      {value}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block">
                Requested handling{" "}
                <select
                  value={kind === "notification" ? "informational" : requirement}
                  disabled={kind === "notification"}
                  onChange={(event) => setRequirement(event.target.value as typeof requirement)}
                  className="rounded border p-2"
                >
                  {requirements.map((value) => (
                    <option key={value} value={value}>
                      {value}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block">
                Target{" "}
                <select
                  value={targetKind}
                  onChange={(event) => setTargetKind(event.target.value as typeof targetKind)}
                  className="rounded border p-2"
                >
                  <option value="inbox">Inbox</option>
                  <option value="task_execution">Current Task execution</option>
                  <option value="exact_run">Exact Run</option>
                </select>
              </label>
              {targetKind !== "inbox" && (
                <>
                  {!thread?.task_id && (
                    <label className="block">
                      Task ID
                      <Input value={taskId} onChange={(event) => setTaskId(event.target.value)} />
                    </label>
                  )}
                  <label className="block">
                    Pipeline version ID
                    <Input
                      value={pipelineId}
                      onChange={(event) => setPipelineId(event.target.value)}
                    />
                  </label>
                  <label className="block">
                    Stage ID
                    <Input value={stageId} onChange={(event) => setStageId(event.target.value)} />
                  </label>
                  <label className="block">
                    Stage visit
                    <Input
                      inputMode="numeric"
                      value={stageVisit}
                      onChange={(event) => setStageVisit(event.target.value)}
                    />
                  </label>
                </>
              )}
              {targetKind === "exact_run" && (
                <>
                  <label className="block">
                    Run ID
                    <Input value={runId} onChange={(event) => setRunId(event.target.value)} />
                  </label>
                  <label className="block">
                    Fencing token
                    <Input
                      inputMode="numeric"
                      value={fence}
                      onChange={(event) => setFence(event.target.value)}
                    />
                  </label>
                  <label className="block">
                    Environment epoch
                    <Input
                      inputMode="numeric"
                      value={epoch}
                      onChange={(event) => setEpoch(event.target.value)}
                    />
                  </label>
                </>
              )}
              <label className="block">
                Message
                <textarea
                  className="w-full rounded border p-2"
                  value={body}
                  onChange={(event) => setBody(event.target.value)}
                  maxLength={32768}
                  rows={4}
                />
              </label>
            </>
          )}
          {action === "waive_message_requirement" && (
            <>
              <p>
                This removes the requirement by management decision. It does not create an Employee
                acknowledgement or answer.
              </p>
              <label className="block">
                Reason
                <Input
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                  maxLength={4096}
                />
              </label>
              <label className="flex gap-2">
                <input type="checkbox" required /> I confirm this waiver
              </label>
            </>
          )}
          <div className="flex gap-2">
            <Button type="submit" disabled={state.status === "sending"}>
              Submit {action.replaceAll("_", " ")}
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={state.status === "sending"}
              onClick={() => setEditing(false)}
            >
              Cancel
            </Button>
          </div>
        </form>
      )}
      {state.status === "sending" && <p role="status">Saving command…</p>}
      {state.status === "unknown" && (
        <p role="alert">
          Outcome unknown. Retry uses the same request and key; refreshing the thread may also
          reveal the saved result.
        </p>
      )}
      {state.status === "unknown" && (
        <Button variant="outline" onClick={() => void submit(state.attempt)}>
          Retry same request
        </Button>
      )}
      {state.status === "refused" && <p role="alert">{state.message}</p>}
      {state.status === "accepted" && (
        <p role="status">
          Saved by Core ({state.receipt.status}). This receipt does not prove delivery or
          acknowledgement.
        </p>
      )}
    </div>
  );
}
