import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { configureProjectHookAttempt, type ProjectHookAttempt } from "../contracts/project-hook.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendConfigureProjectHook } from "./project-hook-command-api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

const template = JSON.stringify(
  {
    name: "",
    image: "runner@sha256:" + "0".repeat(64),
    command: ["/bin/true"],
    workdir: ".",
    limits: {
      cpu_millis: 2000,
      memory_bytes: 1073741824,
      pids: 64,
      wall_seconds: 300,
      stop_grace_seconds: 10,
    },
    max_output_bytes: 1048576,
    applicable_task_kinds: [],
    required: false,
  },
  null,
  2,
);

type Save =
  | { kind: "editing" }
  | { kind: "sending" | "unknown"; attempt: ProjectHookAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; attempt: ProjectHookAttempt; id: string; readError: boolean };

export function ProjectHooksBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [invocationCursor, setInvocationCursor] = useState<string | null>(null);
  const [invocationPrevious, setInvocationPrevious] = useState<(string | null)[]>([]);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(template);
  const [save, setSave] = useState<Save>({ kind: "editing" });
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const key = useMemo(
    () => readKeys.projectHooks(generation, projectId, cursor),
    [generation, projectId, cursor],
  );
  const hooks = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.projectHooks(projectId, cursor, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  const invocationKey = useMemo(
    () => readKeys.projectHookInvocations(generation, projectId, invocationCursor),
    [generation, projectId, invocationCursor],
  );
  const invocations = useQuery({
    queryKey: invocationKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.projectHookInvocations(projectId, invocationCursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(invocationKey);
  const projectKey = useMemo(
    () => ["live", generation, projectId, "hook-create-baseline"] as const,
    [generation, projectId],
  );
  const project = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    enabled: editing,
    retry: false,
  });
  useReadLifetime(projectKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    return () => next.abort();
  }, []);
  const dirty =
    editing && (text !== template || save.kind === "unknown" || save.kind === "sending");
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave hook configuration? The definition and retry key will be lost. A command in flight may have created a version.",
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
  async function readAccepted(attempt: ProjectHookAttempt, id: string) {
    const next = controller.current;
    if (!next) return;
    setSave({ kind: "accepted", attempt, id, readError: false });
    try {
      const fresh = await session.request(generation, (token) =>
        api.project(projectId, token, next.signal),
      );
      if (fresh.revision < attempt.expectedProjectRevision + 1)
        throw new Error("revision readback absent");
      let after: string | null = null;
      let found = false;
      for (let page = 0; page < 20; page += 1) {
        const list = await session.request(generation, (token) =>
          api.projectHooks(projectId, after, token, next.signal),
        );
        found = list.items.some((hook) => hook.id.toLowerCase() === id.toLowerCase());
        if (found || !list.next_cursor) break;
        after = list.next_cursor;
      }
      if (!found) throw new Error("version readback absent");
      if (!current(next)) return;
      queries.setQueryData(["project", generation, projectId], fresh);
      void queries.invalidateQueries({ queryKey: projectKey });
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "project-hooks"],
      });
      setText(template);
      setEditing(false);
      setSave({ kind: "editing" });
      setConfirm(false);
    } catch {
      if (current(next)) setSave({ kind: "accepted", attempt, id, readError: true });
    }
  }
  async function send(attempt: ProjectHookAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setSave({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendConfigureProjectHook(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt, receipt.resource.id);
    } catch (cause) {
      if (current(next))
        setSave(
          cause instanceof LiveCommandError && cause.kind !== "outcome_unknown"
            ? { kind: "refused", error: cause }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  function submit() {
    if (!project.data || project.isFetching) return;
    try {
      const attempt = configureProjectHookAttempt(
        projectId,
        project.data.revision,
        JSON.parse(text) as unknown,
      );
      setError(null);
      setConfirm(false);
      void send(attempt);
    } catch {
      setError("Enter a valid pinned image, command, limits and hook name.");
    }
  }
  function navigate(next: string | null, trail: (string | null)[]) {
    if (editing && !leaveGuard.canLeave()) return;
    prepareReadChange(queries, key);
    setCursor(next);
    setPrevious(trail);
  }
  return (
    <section
      aria-label="Project hook versions"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-lg font-semibold">Project hook versions</h2>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            onClick={() => {
              if (!editing || leaveGuard.canLeave()) setEditing(!editing);
            }}
          >
            {editing ? "Hide configuration" : "Configure hook version"}
          </Button>
          <Button
            variant="outline"
            disabled={hooks.isFetching}
            onClick={() => void hooks.refetch()}
          >
            Refresh hooks
          </Button>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        Each configuration creates a new immutable version. A Pipeline stage must reference its
        exact ID. This view never runs hooks.
      </p>
      {editing && (
        <div className="space-y-3 rounded border border-border p-4">
          <h3 className="font-medium">New hook version</h3>
          <p className="text-xs text-muted-foreground">
            Use a pinned image digest containing the Forge runner. Set required only when this hook
            must pass before completion.
          </p>
          {project.isPending && <p role="status">Loading Project revision…</p>}
          {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
          <label className="block text-sm">
            Hook configuration JSON
            <textarea
              className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
              rows={14}
              value={text}
              disabled={
                save.kind === "sending" || save.kind === "unknown" || save.kind === "accepted"
              }
              onChange={(event) => setText(event.target.value)}
            />
          </label>
          {error && <p role="alert">{error}</p>}
          {(save.kind === "editing" || save.kind === "refused") && !confirm && (
            <Button disabled={!project.data || project.isFetching} onClick={() => setConfirm(true)}>
              Create immutable hook version
            </Button>
          )}
          {confirm && (
            <div
              role="alertdialog"
              aria-label="Confirm hook configuration"
              className="space-y-2 rounded border border-border p-3"
            >
              <p>This saves a new executable hook definition. It does not run the command.</p>
              <Button onClick={submit}>Confirm</Button>
              <Button variant="outline" onClick={() => setConfirm(false)}>
                Cancel
              </Button>
            </div>
          )}
          {save.kind === "sending" && <p role="status">Saving hook version…</p>}
          {save.kind === "unknown" && (
            <div className="space-y-2">
              <p role="alert">Outcome unknown. Retry the exact same request and key.</p>
              <Button onClick={() => void send(save.attempt)}>Retry same request</Button>
            </div>
          )}
          {save.kind === "refused" && (
            <div className="space-y-2">
              <p role="alert">
                Core refused configuration ({save.error.kind}). Keep the definition and refresh the
                Project revision.
              </p>
              <Button onClick={() => void project.refetch()}>Refresh Project</Button>
            </div>
          )}
          {save.kind === "accepted" && (
            <div className="space-y-2">
              <p role="status">
                Hook version accepted.{" "}
                {save.readError ? "Canonical readback failed." : "Reading canonical version…"}
              </p>
              <p className="font-mono text-xs">Version ID: {save.id}</p>
              {save.readError && (
                <Button onClick={() => void readAccepted(save.attempt, save.id)}>
                  Retry version read
                </Button>
              )}
            </div>
          )}
        </div>
      )}
      {hooks.isPending && <p role="status">Loading hook versions…</p>}
      {hooks.isError && (
        <p role="alert">
          {hooks.data ? "Showing stale hook versions. " : ""}
          {describeApiError(hooks.error)}
        </p>
      )}
      {hooks.data?.items.length === 0 && <p>No hook versions configured on this page.</p>}
      {hooks.data && (
        <ul className="space-y-2">
          {hooks.data.items.map((hook) => (
            <li
              key={hook.id}
              className="rounded border border-border p-3 text-sm [overflow-wrap:anywhere]"
            >
              <strong>{hook.name}</strong> <span className="font-mono text-xs">{hook.id}</span>
              <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
                <dt>Required</dt>
                <dd>{hook.required ? "Yes" : "No"}</dd>
                <dt>Task kinds</dt>
                <dd>
                  {hook.applicable_task_kinds.length
                    ? hook.applicable_task_kinds.join(", ")
                    : "All"}
                </dd>
                <dt>Image</dt>
                <dd className="font-mono text-xs">{hook.image}</dd>
                <dt>Command</dt>
                <dd className="font-mono text-xs">{JSON.stringify(hook.command)}</dd>
                <dt>Workdir</dt>
                <dd>{hook.workdir}</dd>
                <dt>Limits</dt>
                <dd className="font-mono text-xs">{JSON.stringify(hook.limits)}</dd>
                <dt>Max output</dt>
                <dd>{hook.max_output_bytes} bytes</dd>
                <dt>Created</dt>
                <dd>{hook.created_at}</dd>
              </dl>
            </li>
          ))}
        </ul>
      )}
      <nav aria-label="Hook pages" className="flex gap-2">
        <Button
          variant="outline"
          disabled={!previous.length}
          onClick={() => navigate(previous.at(-1) ?? null, previous.slice(0, -1))}
        >
          Previous
        </Button>
        <Button
          variant="outline"
          disabled={!hooks.data?.next_cursor}
          onClick={() => {
            if (hooks.data?.next_cursor) navigate(hooks.data.next_cursor, [...previous, cursor]);
          }}
        >
          Next
        </Button>
      </nav>
      <section
        aria-label="Hook invocation results"
        className="space-y-3 border-t border-border pt-4"
      >
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="font-medium">Hook invocation results</h3>
          <Button
            variant="outline"
            disabled={invocations.isFetching}
            onClick={() => void invocations.refetch()}
          >
            Refresh results
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          A skipped result means this hook did not apply to the Task kind. A held result has no
          accepted Pipeline outcome. Run output and private execution settings are not shown.
        </p>
        {invocations.isPending && <p role="status">Loading hook results…</p>}
        {invocations.isError && (
          <p role="alert">
            {invocations.data ? "Showing stale hook results. " : ""}
            {describeApiError(invocations.error)}
          </p>
        )}
        {invocations.data?.items.length === 0 && <p>No hook invocations on this page.</p>}
        {invocations.data && invocations.data.items.length > 0 && (
          <ul className="space-y-2">
            {invocations.data.items.map((item) => (
              <li
                key={item.id}
                className="rounded border border-border p-3 text-sm [overflow-wrap:anywhere]"
              >
                <strong>{item.state}</strong> · {item.verdict ?? "No accepted verdict"}
                <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
                  <dt>Invocation</dt>
                  <dd>{item.id}</dd>
                  <dt>Task</dt>
                  <dd>{item.task_id}</dd>
                  <dt>Pipeline version</dt>
                  <dd>{item.pipeline_version_id}</dd>
                  <dt>Stage visit</dt>
                  <dd>
                    {item.stage_id} #{item.stage_visit}
                  </dd>
                  <dt>Hook version</dt>
                  <dd>{item.hook_version_id}</dd>
                  <dt>Candidate proposal</dt>
                  <dd>{item.candidate_proposal_id ?? "Not applicable"}</dd>
                  <dt>Run</dt>
                  <dd>{item.run_id ?? "No Run (explicit skip)"}</dd>
                  <dt>Mapped outcome</dt>
                  <dd>{item.mapped_outcome ?? "None accepted"}</dd>
                  <dt>Evidence artifact</dt>
                  <dd>{item.artifact_id ?? "None yet"}</dd>
                  <dt>Created</dt>
                  <dd>{item.created_at}</dd>
                  <dt>Updated</dt>
                  <dd>{item.updated_at}</dd>
                </dl>
              </li>
            ))}
          </ul>
        )}
        <nav aria-label="Hook result pages" className="flex gap-2">
          <Button
            variant="outline"
            disabled={!invocationPrevious.length}
            onClick={() => {
              setInvocationCursor(invocationPrevious.at(-1) ?? null);
              setInvocationPrevious(invocationPrevious.slice(0, -1));
            }}
          >
            Previous
          </Button>
          <Button
            variant="outline"
            disabled={!invocations.data?.next_cursor}
            onClick={() => {
              if (invocations.data?.next_cursor) {
                setInvocationCursor(invocations.data.next_cursor);
                setInvocationPrevious([...invocationPrevious, invocationCursor]);
              }
            }}
          >
            Next
          </Button>
        </nav>
      </section>
    </section>
  );
}
