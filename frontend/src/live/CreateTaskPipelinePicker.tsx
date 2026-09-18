import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { ProjectReadScope } from "./read-scope.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

export function CreateTaskPipelinePicker(
  scope: ProjectReadScope & { selected: string; disabled: boolean; onSelect: (id: string) => void },
) {
  const { session, generation, projectId, api } = scope;
  const [navigation, setNavigation] = useState<{
    cursor: string | null;
    previous: (string | null)[];
  }>({ cursor: null, previous: [] });
  const key = useMemo(
    () => ["live", generation, projectId, "create-pipeline-choices", navigation.cursor] as const,
    [generation, projectId, navigation.cursor],
  );
  const versions = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.pipelines(projectId, navigation.cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section
      aria-label="Pipeline version choices"
      className="space-y-3 rounded border border-border p-3"
    >
      <h4 className="font-medium">Pipeline version choices</h4>
      <p className="text-xs">
        Choose an exact version explicitly. Up to 20 versions per page; default and latest are not
        selections.
      </p>
      {versions.isPending && <p role="status">Loading Pipeline choices…</p>}
      {versions.isError && (
        <p role="alert">
          Pipeline choices unavailable. {describeApiError(versions.error, "Pipeline")}
        </p>
      )}
      <Button
        type="button"
        variant="outline"
        disabled={scope.disabled || versions.isFetching}
        onClick={() => void versions.refetch()}
      >
        Refresh pipeline choices
      </Button>
      {versions.error instanceof LiveApiError && versions.error.kind === "cursor_invalid" && (
        <Button
          type="button"
          disabled={scope.disabled}
          onClick={() => {
            if (navigation.cursor === null) void versions.refetch();
            else setNavigation({ cursor: null, previous: [] });
          }}
        >
          Restart Pipeline pagination
        </Button>
      )}
      {versions.isSuccess && versions.data.items.length === 0 && (
        <p>No Pipeline versions on this page.</p>
      )}
      <ul className="space-y-2">
        {versions.data?.items.map((pipeline) => (
          <li key={pipeline.id} className="space-y-1 text-sm [overflow-wrap:anywhere]">
            <p>
              {pipeline.name} · version {pipeline.version} · {pipeline.id}
              {pipeline.deleted_at ? " (deleted)" : ""}
            </p>
            <Button
              type="button"
              variant="outline"
              className="h-auto max-w-full whitespace-normal [overflow-wrap:anywhere]"
              aria-pressed={scope.selected === pipeline.id}
              disabled={
                scope.disabled ||
                !versions.isSuccess ||
                versions.isFetching ||
                pipeline.deleted_at !== null
              }
              onClick={() => scope.onSelect(pipeline.id)}
            >
              Select Pipeline version {pipeline.id}
            </Button>
          </li>
        ))}
      </ul>
      <nav aria-label="Creation Pipeline pagination" className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={scope.disabled || versions.isFetching || navigation.previous.length === 0}
          onClick={() =>
            setNavigation({
              cursor: navigation.previous.at(-1) ?? null,
              previous: navigation.previous.slice(0, -1),
            })
          }
        >
          Previous Pipeline page
        </Button>
        <span>Pipeline page {navigation.previous.length + 1}</span>
        <Button
          type="button"
          variant="outline"
          disabled={
            scope.disabled ||
            versions.isFetching ||
            !versions.isSuccess ||
            versions.data.next_cursor === undefined
          }
          onClick={() => {
            const next = versions.data?.next_cursor;
            if (next !== undefined)
              setNavigation({
                cursor: next,
                previous: [...navigation.previous, navigation.cursor],
              });
          }}
        >
          Next Pipeline page
        </Button>
      </nav>
    </section>
  );
}
