import { useActionState, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { UuidV7Schema } from "../contracts/common.ts";
import { describeApiError } from "./api.ts";
import type { LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";
import { prepareProjectChange } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { TaskBrowser } from "./TaskBrowser.tsx";

type LiveProps = { api: LiveApi; session: LiveSession };

export function LiveApp({ api, session }: LiveProps) {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  return (
    <main className="min-h-screen bg-background px-6 py-12 text-foreground">
      <div className="mx-auto max-w-2xl space-y-6">
        <header className="space-y-2">
          <p className="font-mono text-sm font-semibold tracking-widest text-primary">FORGE</p>
          <h1 className="text-2xl font-semibold">
            {state.status === "authenticated" ? "Live connection" : "Connect to Forge"}
          </h1>
          <p className="text-sm text-muted-foreground">
            Local, read-only Project and Task access. No demo data.
          </p>
        </header>
        {state.notice && (
          <p role="alert" className="rounded-lg border border-border bg-card p-4 text-sm">
            {state.notice}
          </p>
        )}
        {state.status === "authenticated" ? (
          <Connection
            key={state.generation}
            api={api}
            session={session}
            generation={state.generation}
          />
        ) : (
          <Login session={session} disabled={state.status === "storage_unavailable"} />
        )}
      </div>
    </main>
  );
}

function Login({ session, disabled }: { session: LiveSession; disabled: boolean }) {
  const input = useRef<HTMLInputElement>(null);
  const [, connect, pending] = useActionState(async (_state: null, data: FormData) => {
    const code = data.get("code");
    if (input.current) input.current.value = "";
    data.delete("code");
    if (typeof code === "string" && code.length > 0) await session.connect(code);
    return null;
  }, null);
  return (
    <form action={connect} className="space-y-4 rounded-xl border border-border bg-card p-6">
      <div className="space-y-2">
        <label htmlFor="bootstrap-code" className="text-sm font-medium">
          Bootstrap code
        </label>
        <Input
          id="bootstrap-code"
          name="code"
          type="password"
          ref={input}
          required
          autoComplete="off"
          disabled={disabled || pending}
          aria-describedby="code-help"
        />
        <p id="code-help" className="text-xs text-muted-foreground">
          Use the one-time code printed by the local Forge CLI. This tab stores only the resulting
          session.
        </p>
      </div>
      <Button type="submit" disabled={disabled || pending}>
        {pending ? "Connecting…" : "Connect"}
      </Button>
    </form>
  );
}

function Connection({ api, session, generation }: LiveProps & { generation: number }) {
  const queries = useQueryClient();
  const [projectId, setProjectId] = useState<string | null>(null);
  const [idError, setIdError] = useState<string | null>(null);
  const projectKey = useMemo(() => ["project", generation, projectId], [generation, projectId]);
  const health = useQuery({
    queryKey: ["health", generation],
    queryFn: ({ signal }) => session.request(generation, (token) => api.health(token, signal)),
    retry: false,
  });
  const project = useQuery({
    queryKey: projectKey,
    enabled: projectId !== null,
    queryFn: ({ signal }) => {
      if (!projectId) throw new Error("Project ID is missing");
      return session.request(generation, (token) => api.project(projectId, token, signal));
    },
    retry: false,
  });
  useReadLifetime(projectKey);
  const currentProject =
    project.data?.id.toLowerCase() === projectId?.toLowerCase() ? project.data : undefined;
  const [, loadProject] = useActionState((_state: null, data: FormData) => {
    const id = data.get("projectId");
    if (typeof id !== "string" || !UuidV7Schema.safeParse(id.trim()).success) {
      prepareProjectChange(queries, generation);
      setProjectId(null);
      setIdError("Enter a valid UUIDv7 Project ID.");
    } else {
      setIdError(null);
      const next = id.trim();
      if (next === projectId) void project.refetch();
      else {
        prepareProjectChange(queries, generation);
        setProjectId(next);
      }
    }
    return null;
  }, null);

  return (
    <>
      <section
        aria-label="Connection"
        className="space-y-3 rounded-xl border border-border bg-card p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p role="status" className="text-sm">
            {health.isPending
              ? "Checking connection…"
              : health.isSuccess
                ? "Connected to Forge"
                : "Connection unavailable"}
          </p>
          <Button variant="outline" onClick={() => void session.logout()}>
            Log out
          </Button>
        </div>
        {health.isError && (
          <>
            <p role="alert" className="text-sm">
              {describeApiError(health.error)}
            </p>
            <Button
              variant="secondary"
              disabled={health.isFetching}
              onClick={() => void health.refetch()}
            >
              Retry connection
            </Button>
          </>
        )}
      </section>
      <form action={loadProject} className="space-y-4 rounded-xl border border-border bg-card p-6">
        <label htmlFor="project-id" className="text-sm font-medium">
          Project ID
        </label>
        <Input
          id="project-id"
          name="projectId"
          required
          autoComplete="off"
          aria-describedby={idError ? "project-id-error" : undefined}
        />
        {idError && (
          <p id="project-id-error" role="alert" className="text-sm">
            {idError}
          </p>
        )}
        <Button type="submit">Load project</Button>
      </form>
      {projectId && project.isPending && (
        <p role="status" className="text-sm">
          Loading Project…
        </p>
      )}
      {currentProject && project.isFetching && (
        <p role="status" className="text-sm">
          Refreshing Project… Previous data remains visible.
        </p>
      )}
      {projectId && project.isError && (
        <section
          aria-label="Project error"
          className="space-y-3 rounded-xl border border-border p-6"
        >
          <p role="alert">
            {currentProject ? "Showing stale Project. " : ""}
            {describeApiError(project.error)}
          </p>
          <Button
            variant="secondary"
            disabled={project.isFetching}
            onClick={() => void project.refetch()}
          >
            Retry project
          </Button>
        </section>
      )}
      {currentProject && (
        <section
          aria-label="Project"
          className="space-y-4 rounded-xl border border-border bg-card p-6"
        >
          <h2 className="text-lg font-semibold">Project</h2>
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-6 gap-y-3 text-sm">
            <dt className="text-muted-foreground">ID</dt>
            <dd className="break-all font-mono">{currentProject.id}</dd>
            <dt className="text-muted-foreground">Name</dt>
            <dd className="break-words">{currentProject.name}</dd>
            <dt className="text-muted-foreground">Revision</dt>
            <dd>{currentProject.revision}</dd>
            <dt className="text-muted-foreground">Execution gate</dt>
            <dd>{currentProject.execution_gate}</dd>
          </dl>
        </section>
      )}
      {projectId && currentProject && (
        <TaskBrowser
          key={projectId}
          api={api}
          session={session}
          generation={generation}
          projectId={projectId}
        />
      )}
    </>
  );
}
