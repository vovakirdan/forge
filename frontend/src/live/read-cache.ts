import type { QueryClient, QueryKey } from "@tanstack/react-query";

export const readKeys = {
  tasks: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "tasks", cursor] as const,
  task: (generation: number, project: string, task: string) =>
    ["live", generation, project, "task", task] as const,
  pipeline: (generation: number, project: string, task: string, version: string) =>
    ["live", generation, project, "pipeline", task, version] as const,
};

/** Committed scope cleanup owns removal; do not remove a still-observed navigation key. */
export function discardRead(client: QueryClient, key: QueryKey) {
  void client.cancelQueries({ queryKey: key, exact: true });
  client.removeQueries({ queryKey: key, exact: true });
}

/** Cancel immediately, but keep an observed query until the next scope commits. */
export function prepareReadChange(client: QueryClient, key: QueryKey) {
  void client.cancelQueries({ queryKey: key, exact: true });
  client.removeQueries({
    queryKey: key,
    exact: true,
    predicate: (query) => query.getObserversCount() === 0,
  });
}

export function prepareProjectChange(client: QueryClient, generation: number) {
  for (const kind of ["project", "live"]) {
    const queryKey = [kind, generation];
    void client.cancelQueries({ queryKey });
    client.removeQueries({ queryKey, predicate: (query) => query.getObserversCount() === 0 });
  }
}
