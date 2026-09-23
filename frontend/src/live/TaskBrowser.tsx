import { useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError, LiveApiError } from "./api.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { TaskFacts } from "./TaskFacts.tsx";
import { TaskDetailPanel } from "./TaskDetailPanel.tsx";
import type { PriorityCatalog } from "../presentation/priority.ts";
import { TaskCreateEditor } from "./TaskCreateEditor.tsx";
import { TaskPropertySchemaPanel } from "./TaskPropertySchemaPanel.tsx";
import { TaskBoard } from "./TaskBoard.tsx";
import type { TaskSummaryView } from "../contracts/task.ts";

export function TaskBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const [taskId, setTaskId] = useState<string | null>(null);
  const [taskHistory, setTaskHistory] = useState<string[]>([]);
  const [creating, setCreating] = useState(false);
  const [view, setView] = useState<"list" | "board">("list");
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
  const priorityKey = useMemo(
    () => readKeys.priorityScheme(generation, projectId),
    [generation, projectId],
  );
  const priorities = useQuery({
    queryKey: priorityKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.priorityScheme(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(priorityKey);
  const priorityCatalog: PriorityCatalog = priorities.data
    ? { status: "loaded", scheme: priorities.data, stale: priorities.isError }
    : { status: priorities.isPending ? "loading" : "unavailable" };
  function closeTask() {
    if (!scope.leaveGuard.canLeave()) return;
    setTaskId(null);
    setTaskHistory([]);
    if (opener.current?.isConnected) opener.current.focus();
  }
  function navigate(cursor: string | null, previous: (string | null)[]) {
    if (!scope.leaveGuard.canLeave()) return;
    setCreating(false);
    setTaskId(null);
    setTaskHistory([]);
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  function openTask(task: TaskSummaryView, button: HTMLButtonElement) {
    if (taskId === task.id) {
      document.getElementById("task-detail-title")?.focus();
    } else if (scope.leaveGuard.canLeave()) {
      opener.current = button;
      setCreating(false);
      setTaskHistory([]);
      setTaskId(task.id);
    }
  }
  function changeView(next: "list" | "board") {
    if (view === next || !scope.leaveGuard.canLeave()) return;
    setTaskId(null);
    setTaskHistory([]);
    opener.current = null;
    setView(next);
  }
  const items = tasks.data?.items;
  return (
    <>
      <TaskPropertySchemaPanel {...scope} />
      <section aria-label="Tasks" className="space-y-4 rounded-xl border border-border bg-card p-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Tasks</h2>
          <div className="flex flex-wrap gap-2">
            {!creating && (
              <Button
                onClick={() => {
                  if (scope.leaveGuard.canLeave()) {
                    setTaskId(null);
                    setTaskHistory([]);
                    setCreating(true);
                  }
                }}
              >
                Create draft
              </Button>
            )}
            <Button
              variant="outline"
              disabled={priorities.isFetching}
              onClick={() => void priorities.refetch()}
            >
              Refresh priorities
            </Button>
            <Button
              variant="outline"
              disabled={tasks.isFetching}
              onClick={() => void tasks.refetch()}
            >
              Refresh tasks
            </Button>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          Up to 20 Tasks per page in server order. List and Board show only this page, not the
          entire Project. Stage columns use the exact pinned Pipeline version and raw stage ID;
          lifecycle is separate. Run activity is not included in Task summaries. Draft titles and
          descriptions can be edited in Task detail.
        </p>
        <div role="group" aria-label="Task view" className="flex gap-2">
          <Button
            variant={view === "list" ? "default" : "outline"}
            aria-pressed={view === "list"}
            onClick={() => changeView("list")}
          >
            List
          </Button>
          <Button
            variant={view === "board" ? "default" : "outline"}
            aria-pressed={view === "board"}
            onClick={() => changeView("board")}
          >
            Board
          </Button>
        </div>
        {priorities.isPending && <p role="status">Loading priorities…</p>}
        {priorities.isFetching && priorities.data && (
          <p role="status">Refreshing priorities… Previous catalog remains visible.</p>
        )}
        {priorities.isError && (
          <p role="alert">
            {priorities.data ? "Showing stale priorities. " : "Priority names unavailable. "}
            {describeApiError(priorities.error)}
          </p>
        )}
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
        {view === "list" && items && items.length > 0 && (
          <ul className="space-y-3" aria-label="Task list">
            {items.map((task) => (
              <li key={task.id} className="space-y-3 rounded-lg border border-border p-4">
                <button
                  type="button"
                  aria-label={`Open ${task.key}`}
                  aria-controls="task-detail"
                  onClick={(event) => openTask(task, event.currentTarget)}
                  className="w-full cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
                >
                  {task.key} — {task.title}
                </button>
                <TaskFacts task={task} priorities={priorityCatalog} />
              </li>
            ))}
          </ul>
        )}
        {view === "board" && items && items.length > 0 && (
          <TaskBoard tasks={items} priorities={priorityCatalog} onOpenTask={openTask} />
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
      {creating && (
        <TaskCreateEditor
          {...scope}
          onCancel={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false);
            setTaskHistory([]);
            opener.current = null;
            setTaskId(id);
          }}
        />
      )}
      {!creating && taskId && (
        <TaskDetailPanel
          key={taskId}
          {...scope}
          taskId={taskId}
          priorities={priorityCatalog}
          onClose={closeTask}
          onOpenRelated={(id) => {
            if (!scope.leaveGuard.canLeave()) return;
            setTaskHistory([...taskHistory, taskId]);
            setTaskId(id);
          }}
          onBack={
            taskHistory.length === 0
              ? undefined
              : () => {
                  if (!scope.leaveGuard.canLeave()) return;
                  const previous = taskHistory.at(-1);
                  if (previous) {
                    setTaskHistory(taskHistory.slice(0, -1));
                    setTaskId(previous);
                  }
                }
          }
        />
      )}
    </>
  );
}
