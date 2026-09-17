import { useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError, LiveApiError } from "./api.ts";
import type { LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { TaskFacts } from "./TaskFacts.tsx";
import { TaskDetailPanel } from "./TaskDetailPanel.tsx";

export type TaskScope = {
  api: LiveApi;
  session: LiveSession;
  generation: number;
  projectId: string;
};

export function TaskBrowser(scope: TaskScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const [taskId, setTaskId] = useState<string | null>(null);
  const opener = useRef<HTMLButtonElement | null>(null);
  const key = useMemo(
    () => readKeys.tasks(generation, projectId, navigation.cursor),
    [generation, projectId, navigation.cursor],
  );
  const tasks = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.tasks(projectId, navigation.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  function closeTask() {
    setTaskId(null);
    if (opener.current?.isConnected) opener.current.focus();
  }
  function navigate(cursor: string | null, previous: (string | null)[]) {
    setTaskId(null);
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  const items = tasks.data?.items;
  return (
    <>
      <section aria-label="Tasks" className="space-y-4 rounded-xl border border-border bg-card p-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Tasks</h2>
          <Button
            variant="outline"
            disabled={tasks.isFetching}
            onClick={() => void tasks.refetch()}
          >
            Refresh tasks
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          Read-only. Up to 20 Tasks per page; stage IDs and priority IDs are the stored values.
        </p>
        {tasks.isPending && <p role="status">Loading tasks…</p>}
        {tasks.isFetching && items && (
          <p role="status">Refreshing tasks… Previous data remains visible.</p>
        )}
        {tasks.isError && (
          <p role="alert">
            {items ? "Showing stale tasks. " : ""}
            {describeApiError(tasks.error, "Task")}
          </p>
        )}
        {tasks.error instanceof LiveApiError && tasks.error.kind === "cursor_invalid" && (
          <Button
            variant="secondary"
            onClick={() => {
              if (navigation.cursor === null) void tasks.refetch();
              else navigate(null, []);
            }}
          >
            Restart pagination
          </Button>
        )}
        {items?.length === 0 && (
          <p>
            {navigation.cursor === null ? "No tasks in this project." : "No tasks on this page."}
          </p>
        )}
        {items && items.length > 0 && (
          <ul className="space-y-3" aria-label="Task list">
            {items.map((task) => (
              <li key={task.id} className="space-y-3 rounded-lg border border-border p-4">
                <button
                  type="button"
                  aria-label={`Open ${task.key}`}
                  aria-controls="task-detail"
                  onClick={(event) => {
                    opener.current = event.currentTarget;
                    if (taskId === task.id) document.getElementById("task-detail-title")?.focus();
                    else setTaskId(task.id);
                  }}
                  className="w-full cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
                >
                  {task.key} — {task.title}
                </button>
                <TaskFacts task={task} />
              </li>
            ))}
          </ul>
        )}
        <nav
          aria-label="Task pagination"
          className="flex flex-wrap items-center justify-between gap-3"
        >
          <Button
            variant="outline"
            disabled={tasks.isFetching || navigation.previous.length === 0}
            onClick={() => {
              const previous = navigation.previous.slice(0, -1);
              navigate(navigation.previous.at(-1) ?? null, previous);
            }}
          >
            Previous page
          </Button>
          <p className="text-sm">Page {navigation.previous.length + 1}</p>
          <Button
            variant="outline"
            disabled={tasks.isFetching || tasks.isError || tasks.data?.next_cursor === undefined}
            onClick={() => {
              const next = tasks.data?.next_cursor;
              if (next !== undefined) navigate(next, [...navigation.previous, navigation.cursor]);
            }}
          >
            Next page
          </Button>
        </nav>
      </section>
      {taskId && <TaskDetailPanel key={taskId} {...scope} taskId={taskId} onClose={closeTask} />}
    </>
  );
}
