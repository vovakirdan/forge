import type { QueryClient, QueryKey } from "@tanstack/react-query";
import type { DependencyDirection } from "../contracts/task-dependencies.ts";

export const readKeys = {
  employees: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "employees", cursor] as const,
  dependencies: (
    generation: number,
    project: string,
    task: string,
    direction: DependencyDirection,
    cursor: string | null,
  ) => ["live", generation, project, "dependencies", task, direction, cursor] as const,
  priorityScheme: (generation: number, project: string) =>
    ["live", generation, project, "priority-scheme"] as const,
  tasks: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "tasks", cursor] as const,
  task: (generation: number, project: string, task: string) =>
    ["live", generation, project, "task", task] as const,
  pipeline: (generation: number, project: string, task: string, version: string) =>
    ["live", generation, project, "pipeline", task, version] as const,
  pipelineVersions: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "pipeline-versions", cursor] as const,
  pipelineVersion: (generation: number, project: string, version: string) =>
    ["live", generation, project, "pipeline-version", version] as const,
  runs: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "runs", cursor] as const,
  run: (generation: number, project: string, run: string) =>
    ["live", generation, project, "run", run] as const,
};

/** Accepted cancellation can change either side of any dependency in this Project. */
export function invalidateProjectDependencies(
  client: QueryClient,
  generation: number,
  project: string,
) {
  return client.invalidateQueries({ queryKey: ["live", generation, project, "dependencies"] });
}

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

/** Section navigation keeps the Project card, releasing old reads on unmount. */
export function prepareSectionChange(client: QueryClient, generation: number, project: string) {
  const queryKey = ["live", generation, project];
  void client.cancelQueries({ queryKey });
  client.removeQueries({ queryKey, predicate: (query) => query.getObserversCount() === 0 });
}
