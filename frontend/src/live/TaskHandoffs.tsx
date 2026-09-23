import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { Field } from "./Field.tsx";

export function TaskHandoffs({ scope, taskId }: { scope: ProjectReadScope; taskId: string }) {
  const { api, session, generation, projectId } = scope;
  const [cursor, setCursor] = useState<string | null>(null);
  const key = useMemo(
    () => readKeys.taskHandoffs(generation, projectId, taskId, cursor),
    [generation, projectId, taskId, cursor],
  );
  const page = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.taskHandoffs(projectId, taskId, cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section aria-label="Task handoffs" className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <h3 className="font-medium">Task handoffs</h3>
        <Button variant="outline" disabled={page.isFetching} onClick={() => void page.refetch()}>
          Refresh handoffs
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        These are immediate canonical handoff receipts. Summaries and derived memory may arrive
        later. Handoff content is unavailable in this view.
      </p>
      {page.isPending && <p role="status">Loading Task handoffs…</p>}
      {page.isError && <p role="alert">{describeApiError(page.error, "Task")}</p>}
      {page.data?.items.length === 0 && <p>No handoffs on this page.</p>}
      {page.data && page.data.items.length > 0 && (
        <ul className="space-y-2">
          {page.data.items.map((item) => (
            <li key={item.id} className="rounded border border-border p-3 text-sm">
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 break-all">
                <Field label="Handoff">{item.id}</Field>
                <Field label="Kind">{item.kind}</Field>
                <Field label="Source">
                  {item.source_kind}: {item.source_id}
                </Field>
                <Field label="Recorded">{item.created_at}</Field>
                <Field label="Content">Unavailable</Field>
              </dl>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        {cursor && (
          <Button variant="outline" onClick={() => setCursor(null)}>
            First page
          </Button>
        )}
        {page.data?.next_cursor && (
          <Button variant="outline" onClick={() => setCursor(page.data.next_cursor ?? null)}>
            Next page
          </Button>
        )}
      </div>
    </section>
  );
}
