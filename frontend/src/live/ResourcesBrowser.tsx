import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { ResourcesFacts } from "./ResourcesFacts.tsx";
import { useReadLifetime } from "./use-read-lifetime.ts";

export function ResourcesBrowser({ api, session, generation, projectId }: ProjectReadScope) {
  const key = useMemo(() => readKeys.resources(generation, projectId), [generation, projectId]);
  const resources = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.resources(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section
      aria-label="Resources and settings"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">Resources and settings</h2>
        <Button
          variant="outline"
          disabled={resources.isFetching}
          onClick={() => void resources.refetch()}
        >
          Refresh resources
        </Button>
      </div>
      {resources.isPending && <p role="status">Loading resource evidence…</p>}
      {resources.isError && (
        <p role="alert">
          {resources.data ? "Showing stale resource evidence. " : ""}
          {describeApiError(resources.error)}
        </p>
      )}
      {resources.data && <ResourcesFacts snapshot={resources.data} />}
    </section>
  );
}
