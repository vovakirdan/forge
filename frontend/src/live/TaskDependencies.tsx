import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { DependencyDirection } from "../contracts/task-dependencies.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Props = ProjectReadScope & { taskId: string; onOpenRelated: (taskId: string) => void };

export function TaskDependencies(scope: Props) {
  return (
    <>
      <DependencyPanel {...scope} direction="blocked_by" />
      <DependencyPanel {...scope} direction="blocks" />
    </>
  );
}

function DependencyPanel(scope: Props & { direction: DependencyDirection }) {
  const { api, session, generation, projectId, taskId, direction } = scope;
  const queries = useQueryClient();
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const key = useMemo(
    () => readKeys.dependencies(generation, projectId, taskId, direction, navigation.cursor),
    [generation, projectId, taskId, direction, navigation.cursor],
  );
  const dependencies = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.taskDependencies(projectId, taskId, direction, navigation.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  const title = direction === "blocked_by" ? "Depends on" : "Blocks";
  const items = dependencies.data?.items;
  function navigate(cursor: string | null, previous: (string | null)[]) {
    prepareReadChange(queries, key);
    setNavigation({ cursor, previous });
  }
  return (
    <section aria-label={title} className="space-y-3 rounded-lg border border-border p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="font-medium">{title}</h3>
        <Button
          variant="outline"
          disabled={dependencies.isFetching}
          onClick={() => void dependencies.refetch()}
        >
          Refresh dependencies
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Saved links remain visible after their condition is satisfied. One satisfied condition does
        not prove Task readiness.
      </p>
      {dependencies.isPending && <p role="status">Loading dependencies…</p>}
      {dependencies.isFetching && items && (
        <p role="status">Refreshing dependencies… Previous data remains visible.</p>
      )}
      {dependencies.isError && (
        <>
          <p role="alert">
            {items ? "Showing stale dependencies. " : ""}
            {describeApiError(dependencies.error, "Task")}
          </p>
          <Button
            variant="secondary"
            disabled={dependencies.isFetching}
            onClick={() => void dependencies.refetch()}
          >
            Retry dependencies
          </Button>
        </>
      )}
      {dependencies.error instanceof LiveApiError &&
        dependencies.error.kind === "cursor_invalid" && (
          <Button
            variant="secondary"
            onClick={() =>
              navigation.cursor === null ? void dependencies.refetch() : navigate(null, [])
            }
          >
            Restart dependency pagination
          </Button>
        )}
      {items?.length === 0 && (
        <p>
          {navigation.cursor === null
            ? "No dependencies in this direction."
            : "No dependencies on this page."}
        </p>
      )}
      {items && items.length > 0 && (
        <ul aria-label={`${title} list`} className="space-y-3">
          {items.map((item) => (
            <li key={item.related_task.id} className="space-y-2 rounded border border-border p-3">
              <button
                type="button"
                aria-label={`Open related ${item.related_task.key}`}
                aria-controls="task-detail"
                onClick={() => scope.onOpenRelated(item.related_task.id)}
                className="w-full cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
              >
                {item.related_task.key} — {item.related_task.title}
              </button>
              <p className="text-sm">Lifecycle: {item.related_task.lifecycle}</p>
              <p className="text-sm">Blocker must be done (task_done)</p>
              <p className="text-sm">
                Condition:{" "}
                {
                  {
                    pending: "Pending",
                    satisfied: "Satisfied",
                    blocker_cancelled: "Blocker cancelled",
                  }[item.condition_state]
                }
              </p>
            </li>
          ))}
        </ul>
      )}
      <nav
        aria-label={`${title} pagination`}
        className="flex flex-wrap items-center justify-between gap-3"
      >
        <Button
          variant="outline"
          disabled={dependencies.isFetching || navigation.previous.length === 0}
          onClick={() =>
            navigate(navigation.previous.at(-1) ?? null, navigation.previous.slice(0, -1))
          }
        >
          Previous dependencies
        </Button>
        <p className="text-sm">Page {navigation.previous.length + 1}</p>
        <Button
          variant="outline"
          disabled={
            dependencies.isFetching ||
            dependencies.isError ||
            dependencies.data?.next_cursor === undefined
          }
          onClick={() => {
            const next = dependencies.data?.next_cursor;
            if (next !== undefined) navigate(next, [...navigation.previous, navigation.cursor]);
          }}
        >
          Next dependencies
        </Button>
      </nav>
    </section>
  );
}
