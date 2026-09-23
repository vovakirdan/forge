import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { TaskPropertySchemaEditor } from "./TaskPropertySchemaEditor.tsx";

export function TaskPropertySchemaPanel({
  api,
  session,
  generation,
  projectId,
  leaveGuard,
}: ProjectReadScope) {
  const key = useMemo(
    () => readKeys.taskPropertySchema(generation, projectId),
    [generation, projectId],
  );
  const schema = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.taskPropertySchema(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  const definitions = schema.data ? Object.values(schema.data.schema.definitions) : [];
  return (
    <section
      aria-label="Task properties"
      className="space-y-3 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-semibold">Task properties</h2>
        <Button
          variant="outline"
          disabled={schema.isFetching}
          onClick={() => void schema.refetch()}
        >
          Refresh schema
        </Button>
      </div>
      {schema.isPending && <p role="status">Loading property schema…</p>}
      {schema.isError && (
        <p role="alert">
          {schema.data ? "Showing stale property schema. " : ""}
          {describeApiError(schema.error)}
        </p>
      )}
      {schema.data && (
        <p className="text-xs text-muted-foreground">
          Project schema revision {schema.data.project_revision}. Properties are validated by Core
          at Task approval.
        </p>
      )}
      {schema.data && definitions.length === 0 && (
        <p className="text-sm">No Task properties configured in this Project.</p>
      )}
      {definitions.length > 0 && (
        <ul className="space-y-2 text-sm">
          {definitions.map((definition) => (
            <li key={definition.key} className="rounded border border-border p-3">
              <strong>{definition.display_name}</strong> <code>{definition.key}</code>
              <p className="text-xs text-muted-foreground">
                {definition.property_type} · {definition.required ? "required" : "optional"}
                {definition.allowed_choices
                  ? ` · choices: ${definition.allowed_choices.join(", ")}`
                  : ""}
              </p>
            </li>
          ))}
        </ul>
      )}
      {schema.data && (
        <TaskPropertySchemaEditor
          {...{ api, session, generation, projectId, leaveGuard }}
          baseline={schema.data}
          queryKey={key}
        />
      )}
    </section>
  );
}
