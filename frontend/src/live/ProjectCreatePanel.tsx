import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  createProjectAttempt,
  reserveProjectId,
  type CreateProjectAttempt,
} from "../contracts/create-project.ts";
import { LiveApiError, type LiveApi } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendCreateProject } from "./create-project-api.ts";
import type { LeaveGuard } from "./leave-guard.ts";
import type { LiveSession } from "./session.ts";

type Props = {
  api: LiveApi;
  session: LiveSession;
  generation: number;
  leaveGuard: LeaveGuard;
  onClose: () => void;
  onSelect: (id: string) => void;
};
type State =
  | { kind: "editing" }
  | { kind: "sending" | "unknown"; attempt: CreateProjectAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; readError: boolean; attempt: CreateProjectAttempt };

export function ProjectCreatePanel({
  api,
  session,
  generation,
  leaveGuard,
  onClose,
  onSelect,
}: Props) {
  const queries = useQueryClient();
  const [projectId] = useState(() => reserveProjectId());
  const [name, setName] = useState("");
  const [state, setState] = useState<State>({ kind: "editing" });
  const [fieldError, setFieldError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const titleRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    titleRef.current?.focus();
    return () => next.abort();
  }, []);
  const dirty =
    state.kind !== "accepted" &&
    (name.length > 0 || state.kind === "sending" || state.kind === "unknown");
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Project creation? The draft name and retry key will be lost. An in-flight command may still have created the Project.",
    );
    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [dirty, leaveGuard]);
  function current(next: AbortController) {
    return !next.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function readAccepted(attempt: CreateProjectAttempt) {
    const next = controller.current;
    if (!next) return;
    setState({ kind: "accepted", attempt, readError: false });
    try {
      const project = await session.request(generation, (token) =>
        api.project(attempt.projectId, token, next.signal),
      );
      if (!current(next)) return;
      if (project.revision !== 1 || project.name !== attempt.name)
        throw new Error("Project readback did not match receipt");
      queries.setQueryData(["project", generation, attempt.projectId], project);
      void queries.invalidateQueries({ queryKey: ["projects", generation] });
      setState({ kind: "accepted", attempt, readError: false });
    } catch {
      if (current(next)) setState({ kind: "accepted", attempt, readError: true });
    }
  }
  async function send(attempt: CreateProjectAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt });
    try {
      await session.request(generation, (token) =>
        sendCreateProject(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt);
    } catch (error) {
      if (current(next))
        setState(
          error instanceof LiveCommandError && error.kind !== "outcome_unknown"
            ? { kind: "refused", error }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  function submit() {
    try {
      const attempt = createProjectAttempt(projectId, name);
      setFieldError(null);
      void send(attempt);
    } catch {
      setFieldError("Enter a nonblank Project name of at most 240 characters.");
    }
  }
  return (
    <section aria-label="Create Project" className="space-y-3 rounded border border-border p-4">
      <h3 ref={titleRef} tabIndex={-1} className="font-semibold">
        Create Project
      </h3>
      <p className="text-xs text-muted-foreground">
        The new Project starts at revision 1 with execution stopped. Its ID is reserved across
        retries.
      </p>
      <p className="font-mono text-xs [overflow-wrap:anywhere]">Project ID: {projectId}</p>
      <label className="block text-sm">
        Project name
        <Input
          value={name}
          onChange={(event) => setName(event.target.value)}
          disabled={state.kind !== "editing" && state.kind !== "refused"}
        />
      </label>
      {fieldError && <p role="alert">{fieldError}</p>}
      {(state.kind === "editing" || state.kind === "refused") && (
        <Button onClick={submit}>Create Project</Button>
      )}
      {state.kind === "sending" && <p role="status">Creating Project…</p>}
      {state.kind === "unknown" && (
        <div className="space-y-2">
          <p role="alert">
            Outcome unknown. Retry the same request with its original name and key.
          </p>
          <Button onClick={() => void send(state.attempt)}>Retry same request</Button>
        </div>
      )}
      {state.kind === "refused" && (
        <p role="alert">
          {state.error.kind === "idempotency_conflict"
            ? "This request key belongs to a different command. Review the name before trying again."
            : `Core refused creation (${state.error.kind}). Check the name and try again.`}
        </p>
      )}
      {state.kind === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Project creation accepted.{" "}
            {state.readError ? "Canonical readback failed." : "Canonical Project confirmed."}
          </p>
          {state.readError ? (
            <Button onClick={() => void readAccepted(state.attempt)}>Retry Project read</Button>
          ) : (
            <Button onClick={() => onSelect(projectId)}>Open created Project</Button>
          )}
        </div>
      )}
      <Button
        variant="outline"
        onClick={() => {
          if (leaveGuard.canLeave()) onClose();
        }}
      >
        Close
      </Button>
    </section>
  );
}
