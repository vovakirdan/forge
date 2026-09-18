import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { TaskDetailView } from "../contracts/task.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { Field } from "./Field.tsx";
import { Button } from "../components/ui/button.tsx";

export function TaskCancellationFacts({
  api,
  session,
  generation,
  projectId,
  cancellation,
}: ProjectReadScope & { cancellation: NonNullable<TaskDetailView["cancellation"]> }) {
  const key = useMemo(
    () => ["live", generation, projectId, "cancellation-reasons"] as const,
    [generation, projectId],
  );
  const catalog = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.cancellationReasons(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  const reason = catalog.isSuccess
    ? catalog.data.reasons.find((entry) => entry.id === cancellation.reason_id)
    : undefined;
  return (
    <section aria-label="Cancellation" className="space-y-2">
      <h3 className="font-medium">Cancellation</h3>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <Field label="Reason">
          {reason
            ? `${reason.display_name} (${reason.id})${reason.retired ? " — retired" : ""}`
            : cancellation.reason_id}
        </Field>
        <Field label="Note">{cancellation.note ?? "Not provided."}</Field>
        <Field label="Cancelled by">
          {cancellation.cancelled_by.kind}: {cancellation.cancelled_by.id}
        </Field>
        <Field label="Cancelled at">{cancellation.cancelled_at}</Field>
      </dl>
      {catalog.isError && (
        <>
          <p role="alert">Cancellation reason catalog unavailable. The saved reason ID is shown.</p>
          <Button disabled={catalog.isFetching} onClick={() => void catalog.refetch()}>
            Retry cancellation reasons
          </Button>
        </>
      )}
      <p className="text-xs">Cancellation does not prove that active Run processes have stopped.</p>
    </section>
  );
}
