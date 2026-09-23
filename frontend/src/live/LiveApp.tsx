import { useActionState, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  Activity,
  BookOpen,
  Bot,
  Cog,
  Columns3,
  Gauge,
  GitBranch,
  Shield,
  Sparkles,
} from "lucide-react";
import { APP_NAME } from "../config/app.ts";
import { describeApiError } from "./api.ts";
import type { LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";
import { prepareProjectChange, prepareSectionChange } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { ProjectReads, type ProjectSection } from "./ProjectReads.tsx";
import { ProjectPicker } from "./ProjectPicker.tsx";
import { createLeaveGuard } from "./leave-guard.ts";
import { EventBridge } from "./EventBridge.tsx";

type LiveProps = { api: LiveApi; session: LiveSession };
const sections = [
  { id: "tasks", label: "Tasks", icon: Columns3 },
  { id: "team", label: "Team", icon: Bot },
  { id: "runs", label: "Runs", icon: Activity },
  { id: "activity", label: "Activity", icon: Activity },
  { id: "pipelines", label: "Pipeline versions", icon: GitBranch },
  { id: "knowledge", label: "Knowledge", icon: BookOpen },
  { id: "system-jobs", label: "System Jobs", icon: Cog },
  { id: "resources", label: "Resources", icon: Gauge },
  { id: "management", label: "Management", icon: Shield },
] as const satisfies ReadonlyArray<{
  id: ProjectSection;
  label: string;
  icon: typeof Columns3;
}>;

export function LiveApp({ api, session }: LiveProps) {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  if (state.status === "authenticated")
    return (
      <Connection
        key={state.generation}
        api={api}
        session={session}
        generation={state.generation}
        notice={state.notice}
      />
    );
  return (
    <main className="grid min-h-screen place-items-center bg-background px-4 py-12 text-foreground">
      <div className="w-full max-w-md space-y-6">
        <header className="space-y-3">
          <div className="flex items-center gap-2 text-primary">
            <span className="grid size-8 place-items-center rounded-md bg-primary text-primary-foreground">
              <Sparkles className="size-4" aria-hidden="true" />
            </span>
            <span className="font-semibold tracking-tight">{APP_NAME}</span>
          </div>
          <h1 className="text-2xl font-semibold">Connect to Forge</h1>
          <p className="text-sm text-muted-foreground">Local owner access to Control Room.</p>
        </header>
        {state.notice && (
          <p role="alert" className="rounded-lg border border-border bg-card p-4 text-sm">
            {state.notice}
          </p>
        )}
        <Login session={session} disabled={state.status === "storage_unavailable"} />
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

function Connection({
  api,
  session,
  generation,
  notice,
}: LiveProps & { generation: number; notice?: string | null }) {
  const queries = useQueryClient();
  const [leaveGuard] = useState(() => createLeaveGuard((message) => window.confirm(message)));
  const [projectId, setProjectId] = useState<string | null>(null);
  const [section, setSection] = useState<ProjectSection>("tasks");
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
  function selectProject(next: string) {
    if (next === projectId) {
      void project.refetch();
      return;
    }
    if (!leaveGuard.canLeave()) return;
    prepareProjectChange(queries, generation);
    setSection("tasks");
    setProjectId(next);
  }
  function selectSection(next: ProjectSection) {
    if (next === section || !projectId || !currentProject) return;
    if (!leaveGuard.canLeave()) return;
    prepareSectionChange(queries, generation, projectId);
    setSection(next);
  }

  return (
    <div className="min-h-screen bg-background text-foreground md:grid md:grid-cols-[216px_minmax(0,1fr)]">
      <aside className="flex flex-col border-b border-sidebar-border bg-sidebar md:sticky md:top-0 md:h-screen md:border-b-0 md:border-r">
        <div className="flex h-11 items-center gap-2 border-b border-sidebar-border px-3">
          <span className="grid size-5 place-items-center rounded bg-primary text-primary-foreground">
            <Sparkles className="size-3" aria-hidden="true" />
          </span>
          <span className="text-[13px] font-semibold tracking-tight">{APP_NAME}</span>
        </div>
        <div className="border-b border-sidebar-border px-3 py-3">
          <p className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Current Project
          </p>
          <p className="truncate text-[12.5px] font-medium" title={currentProject?.name}>
            {currentProject?.name ?? "Choose a Project"}
          </p>
        </div>
        <nav
          aria-label="Project sections"
          className="flex gap-1 overflow-x-auto p-2 md:flex-1 md:flex-col"
        >
          {sections.map(({ id, label, icon: Icon }) => (
            <Button
              key={id}
              variant="ghost"
              aria-pressed={section === id}
              disabled={!currentProject}
              className={`h-8 shrink-0 justify-start gap-2 px-2 text-[12.5px] md:w-full ${
                section === id && currentProject
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "text-sidebar-foreground"
              }`}
              onClick={() => selectSection(id)}
            >
              <Icon className="size-3.5" aria-hidden="true" />
              {label}
            </Button>
          ))}
        </nav>
        <p className="hidden border-t border-sidebar-border px-3 py-3 text-[11px] text-muted-foreground md:block">
          Local owner session · Core data
        </p>
      </aside>
      <div className="min-w-0">
        <header className="flex min-h-11 flex-wrap items-center gap-3 border-b border-border bg-surface px-4 py-2">
          <div className="min-w-0 flex-1">
            <h1 className="truncate text-sm font-semibold">Control Room</h1>
            <p className="truncate text-xs text-muted-foreground">
              {currentProject?.name ?? "Select a Project to begin"}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              if (leaveGuard.canLeave()) void session.logout();
            }}
          >
            Log out
          </Button>
        </header>
        <main className="mx-auto max-w-6xl space-y-5 px-4 py-6 md:px-6">
          {notice && (
            <p role="alert" className="rounded-lg border border-border bg-card p-4 text-sm">
              {notice}
            </p>
          )}
          <section
            aria-label="Connection"
            className="space-y-3 rounded-xl border border-border bg-card p-4"
          >
            <p role="status" className="text-sm">
              {health.isPending
                ? "Checking connection…"
                : health.isSuccess
                  ? "Connected to Forge"
                  : "Connection unavailable"}
            </p>
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
          <ProjectPicker
            api={api}
            session={session}
            generation={generation}
            selectedId={projectId}
            onSelect={selectProject}
            leaveGuard={leaveGuard}
          />
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
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h2 className="text-lg font-semibold">Project</h2>
                <Button
                  variant="outline"
                  disabled={project.isFetching}
                  onClick={() => void project.refetch()}
                >
                  Refresh project
                </Button>
              </div>
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
            <EventBridge
              key={`events-${projectId}`}
              api={api}
              session={session}
              generation={generation}
              projectId={projectId}
              visible={section === "activity"}
              leaveGuard={leaveGuard}
            />
          )}
          {projectId && currentProject && section !== "activity" && (
            <ProjectReads
              key={`reads-${projectId}`}
              api={api}
              session={session}
              generation={generation}
              projectId={projectId}
              leaveGuard={leaveGuard}
              section={section}
            />
          )}
        </main>
      </div>
    </div>
  );
}
