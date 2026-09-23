import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  createPipelineAttempt,
  InitialPipelineDefinition,
  type CreatePipelineAttempt,
} from "../contracts/create-pipeline.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendCreatePipeline } from "./create-pipeline-api.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Props = ProjectReadScope & { onClose: () => void; onApplied: (pipelineId: string) => void };
type State =
  | { kind: "editing" }
  | { kind: "sending" | "unknown"; attempt: CreatePipelineAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; attempt: CreatePipelineAttempt; pipelineId: string; readError: boolean };

export function PipelineCreatePanel({
  api,
  session,
  generation,
  projectId,
  leaveGuard,
  onClose,
  onApplied,
}: Props) {
  const queries = useQueryClient();
  const [name, setName] = useState("");
  const [definitionText, setDefinitionText] = useState(
    JSON.stringify(InitialPipelineDefinition, null, 2),
  );
  const [state, setState] = useState<State>({ kind: "editing" });
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const confirmRef = useRef<HTMLHeadingElement>(null);
  const projectKey = useMemo(
    () => ["live", generation, projectId, "pipeline-create-baseline"] as const,
    [generation, projectId],
  );
  const project = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(projectKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    titleRef.current?.focus();
    return () => next.abort();
  }, []);
  useEffect(() => {
    if (confirm) confirmRef.current?.focus();
  }, [confirm]);
  const dirty =
    state.kind !== "accepted" &&
    (name.length > 0 ||
      definitionText !== JSON.stringify(InitialPipelineDefinition, null, 2) ||
      state.kind === "sending" ||
      state.kind === "unknown");
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Pipeline creation? The graph and retry key will be lost. An in-flight command may still have created the Pipeline.",
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
  async function readAccepted(attempt: CreatePipelineAttempt, pipelineId: string) {
    const next = controller.current;
    if (!next) return;
    setState({ kind: "accepted", attempt, pipelineId, readError: false });
    try {
      const fresh = await session.request(generation, (token) =>
        api.project(projectId, token, next.signal),
      );
      if (fresh.revision < attempt.expectedProjectRevision + 1)
        throw new Error("Project revision did not advance");
      let cursor: string | null = null;
      let found: Awaited<ReturnType<typeof api.pipelineCatalog>>["items"][number] | undefined;
      for (let page = 0; page < 20; page += 1) {
        const list = await session.request(generation, (token) =>
          api.pipelineCatalog(projectId, cursor, token, next.signal),
        );
        found = list.items.find((item) => item.id.toLowerCase() === pipelineId.toLowerCase());
        if (found || !list.next_cursor) break;
        cursor = list.next_cursor;
      }
      if (
        !found ||
        found.revision !== 1 ||
        !found.latest_version_id ||
        found.default_version_id !== found.latest_version_id ||
        found.deleted_at
      )
        throw new Error("New Pipeline catalog readback absent");
      const version = await session.request(generation, (token) =>
        api.pipeline(projectId, found.latest_version_id!, token, next.signal),
      );
      if (version.pipeline_id.toLowerCase() !== pipelineId.toLowerCase() || version.version !== 1)
        throw new Error("First Pipeline graph readback absent");
      if (!current(next)) return;
      queries.setQueryData(["project", generation, projectId], fresh);
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "pipeline-catalog"],
      });
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "pipeline-versions"],
      });
      onApplied(pipelineId);
    } catch {
      if (current(next)) setState({ kind: "accepted", attempt, pipelineId, readError: true });
    }
  }
  async function send(attempt: CreatePipelineAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendCreatePipeline(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt, receipt.resource.id);
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
    if (!project.data || project.isFetching) return;
    try {
      const graph = JSON.parse(definitionText) as unknown;
      const attempt = createPipelineAttempt(projectId, project.data.revision, name, graph);
      setFieldError(null);
      setConfirm(false);
      void send(attempt);
    } catch {
      setFieldError("Enter a Pipeline name and a complete valid graph JSON definition.");
    }
  }
  const editing = state.kind === "editing" || state.kind === "refused";
  return (
    <section
      aria-label="Create Pipeline"
      className="space-y-3 rounded-xl border border-border bg-card p-5"
    >
      <h3 ref={titleRef} tabIndex={-1} className="font-semibold">
        Create Pipeline
      </h3>
      <p className="text-xs text-muted-foreground">
        The graph becomes immutable version 1. Names, stage IDs, outcomes, transitions, workspace
        and optional hooks are validated by Core.
      </p>
      {project.isPending && <p role="status">Loading Project revision…</p>}
      {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
      <label className="block text-sm">
        Pipeline name
        <Input value={name} onChange={(event) => setName(event.target.value)} disabled={!editing} />
      </label>
      <label className="block text-sm">
        Complete Pipeline graph JSON
        <textarea
          value={definitionText}
          onChange={(event) => setDefinitionText(event.target.value)}
          disabled={!editing}
          rows={14}
          className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
        />
      </label>
      {fieldError && <p role="alert">{fieldError}</p>}
      {editing && !confirm && (
        <Button disabled={!project.data || project.isFetching} onClick={() => setConfirm(true)}>
          Create immutable Pipeline
        </Button>
      )}
      {confirm && (
        <div
          role="alertdialog"
          aria-labelledby="create-pipeline-confirm"
          className="space-y-2 rounded border border-border p-3"
        >
          <h4 id="create-pipeline-confirm" tabIndex={-1} ref={confirmRef}>
            Confirm Pipeline publication
          </h4>
          <p className="text-sm">This creates a catalog entry and its immutable first version.</p>
          <Button onClick={submit}>Confirm</Button>
          <Button variant="outline" onClick={() => setConfirm(false)}>
            Cancel
          </Button>
        </div>
      )}
      {state.kind === "sending" && <p role="status">Creating Pipeline…</p>}
      {state.kind === "unknown" && (
        <div className="space-y-2">
          <p role="alert">
            Outcome unknown. Retry the exact same graph and key to obtain its receipt.
          </p>
          <Button onClick={() => void send(state.attempt)}>Retry same request</Button>
        </div>
      )}
      {state.kind === "refused" && (
        <div className="space-y-2">
          <p role="alert">
            Core refused creation ({state.error.kind}). Keep your graph and refresh the Project
            revision before trying again.
          </p>
          <Button onClick={() => void project.refetch()}>Refresh Project</Button>
        </div>
      )}
      {state.kind === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Pipeline creation accepted.{" "}
            {state.readError ? "Canonical readback failed." : "Reading canonical graph…"}
          </p>
          <p className="font-mono text-xs">Pipeline ID: {state.pipelineId}</p>
          {state.readError && (
            <Button onClick={() => void readAccepted(state.attempt, state.pipelineId)}>
              Retry catalog read
            </Button>
          )}
        </div>
      )}
      <Button
        variant="outline"
        onClick={() => {
          if (leaveGuard.canLeave()) onClose();
        }}
      >
        Close editor
      </Button>
    </section>
  );
}
