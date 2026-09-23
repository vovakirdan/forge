import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError, LiveApiError, type LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";
import { prepareReadChange } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { ProjectCreatePanel } from "./ProjectCreatePanel.tsx";
import type { LeaveGuard } from "./leave-guard.ts";

export function ProjectPicker({
  api,
  session,
  generation,
  selectedId,
  onSelect,
  leaveGuard,
}: {
  api: LiveApi;
  session: LiveSession;
  generation: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
  leaveGuard: LeaveGuard;
}) {
  const queries = useQueryClient();
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [creating, setCreating] = useState(false);
  const key = useMemo(() => ["projects", generation, cursor] as const, [generation, cursor]);
  const projects = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.projects(cursor, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  function navigate(next: string | null, trail: (string | null)[]) {
    if (creating && !leaveGuard.canLeave()) return;
    prepareReadChange(queries, key);
    setCursor(next);
    setPrevious(trail);
  }
  return (
    <section
      aria-label="Project selector"
      className="space-y-3 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">Choose Project</h2>
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() => {
              if (!creating || leaveGuard.canLeave()) setCreating(!creating);
            }}
          >
            {creating ? "Hide creation" : "Create Project"}
          </Button>
          <Button
            variant="outline"
            disabled={projects.isFetching}
            onClick={() => void projects.refetch()}
          >
            Refresh Projects
          </Button>
        </div>
      </div>
      {creating && (
        <ProjectCreatePanel
          api={api}
          session={session}
          generation={generation}
          leaveGuard={leaveGuard}
          onClose={() => setCreating(false)}
          onSelect={(id) => {
            onSelect(id);
            setCreating(false);
          }}
        />
      )}
      {projects.isPending && <p role="status">Loading Projects…</p>}
      {projects.isFetching && projects.data && <p role="status">Refreshing Projects…</p>}
      {projects.isError && (
        <>
          <p role="alert">
            {projects.data ? "Showing stale Projects. " : ""}
            {describeApiError(projects.error)}
          </p>
          <Button
            variant="secondary"
            disabled={projects.isFetching}
            onClick={() => void projects.refetch()}
          >
            Retry Projects
          </Button>
        </>
      )}
      {projects.error instanceof LiveApiError && projects.error.kind === "cursor_invalid" && (
        <Button variant="secondary" onClick={() => navigate(null, [])}>
          Restart Project pages
        </Button>
      )}
      {projects.data?.items.length === 0 && <p>No Projects on this page.</p>}
      {projects.data && projects.data.items.length > 0 && (
        <ul className="space-y-2" aria-label="Projects">
          {projects.data.items.map((project) => (
            <li key={project.id}>
              <Button
                variant={selectedId === project.id ? "default" : "outline"}
                aria-pressed={selectedId === project.id}
                className="h-auto max-w-full whitespace-normal text-left [overflow-wrap:anywhere]"
                disabled={projects.isFetching}
                onClick={() => onSelect(project.id)}
              >
                Open project {project.name} ({project.id})
              </Button>
            </li>
          ))}
        </ul>
      )}
      <nav aria-label="Project pages" className="flex flex-wrap items-center justify-between gap-3">
        <Button
          variant="outline"
          disabled={projects.isFetching || previous.length === 0}
          onClick={() => navigate(previous.at(-1) ?? null, previous.slice(0, -1))}
        >
          Previous Projects
        </Button>
        <span>Page {previous.length + 1}</span>
        <Button
          variant="outline"
          disabled={projects.isFetching || projects.isError || !projects.data?.next_cursor}
          onClick={() => {
            if (projects.data?.next_cursor)
              navigate(projects.data.next_cursor, [...previous, cursor]);
          }}
        >
          Next Projects
        </Button>
      </nav>
    </section>
  );
}
