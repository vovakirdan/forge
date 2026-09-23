import type { QueryClient, QueryKey } from "@tanstack/react-query";
import type { DependencyDirection } from "../contracts/task-dependencies.ts";

export const readKeys = {
  recoveryAssessmentReadiness: (generation: number, project: string, run: string) =>
    ["live", generation, project, "recovery-assessment-readiness", run] as const,
  managementFactsPage: (generation: number, project: string, kind: string, cursor: string | null) =>
    ["live", generation, project, "management-facts", kind, cursor] as const,
  recoverySettings: (generation: number, project: string) =>
    ["live", generation, project, "recovery-settings"] as const,
  managementPage: (generation: number, project: string, kind: string, cursor: string | null) =>
    ["live", generation, project, "management", kind, cursor] as const,
  escalationDetail: (generation: number, project: string, escalation: string) =>
    ["live", generation, project, "escalation", escalation] as const,
  resources: (generation: number, project: string) =>
    ["live", generation, project, "resources"] as const,
  taskPropertySchema: (generation: number, project: string) =>
    ["live", generation, project, "task-property-schema"] as const,
  pipelineCatalog: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "pipeline-catalog", cursor] as const,
  projectHooks: (generation: number, project: string, after: string | null) =>
    ["live", generation, project, "project-hooks", after] as const,
  projectHookInvocations: (generation: number, project: string, after: string | null) =>
    ["live", generation, project, "hook-invocations", after] as const,
  knowledgePages: (generation: number, project: string, after: string | null) =>
    ["live", generation, project, "knowledge-pages", after] as const,
  knowledgePage: (generation: number, project: string, page: string) =>
    ["live", generation, project, "knowledge-page", page] as const,
  knowledgeHistory: (generation: number, project: string, page: string, after: string | null) =>
    ["live", generation, project, "knowledge-history", page, after] as const,
  memoryEntries: (
    generation: number,
    project: string,
    employee: string | null,
    after: string | null,
  ) => ["live", generation, project, "memory-entries", employee, after] as const,
  memoryEntry: (generation: number, project: string, entry: string) =>
    ["live", generation, project, "memory-entry", entry] as const,
  memoryHistory: (generation: number, project: string, entry: string, after: string | null) =>
    ["live", generation, project, "memory-history", entry, after] as const,
  memorySearch: (generation: number, project: string, employee: string | null, term: string) =>
    ["live", generation, project, "memory-search", employee, term] as const,
  memoryStatus: (generation: number, project: string) =>
    ["live", generation, project, "memory-status"] as const,
  systemJobs: (generation: number, project: string) =>
    ["live", generation, project, "system-jobs"] as const,
  systemJobAttempts: (generation: number, project: string, job: string, cursor: string | null) =>
    ["live", generation, project, "system-job-attempts", job, cursor] as const,
  employeeOnboarding: (generation: number, project: string, employee: string) =>
    ["live", generation, project, "employee-onboarding", employee] as const,
  employees: (generation: number, project: string, cursor: string | null) =>
    ["live", generation, project, "employees", cursor] as const,
  employee: (generation: number, project: string, employee: string) =>
    ["live", generation, project, "employee", employee] as const,
  employeeOperations: (generation: number, project: string, employee: string) =>
    ["live", generation, project, "employee-operations", employee] as const,
  employeeRuntimeMetadata: (generation: number, project: string, employee: string) =>
    ["live", generation, project, "employee-runtime-metadata", employee] as const,
  employeeRuns: (generation: number, project: string, employee: string, cursor: string | null) =>
    ["live", generation, project, "employee-runs", employee, cursor] as const,
  employeeThreads: (generation: number, project: string, employee: string, after: string | null) =>
    ["live", generation, project, "employee-threads", employee, after] as const,
  employeeMessages: (
    generation: number,
    project: string,
    employee: string,
    thread: string,
    after: string | null,
  ) => ["live", generation, project, "employee-messages", employee, thread, after] as const,
  messageDelivery: (generation: number, project: string, thread: string, after: string | null) =>
    ["live", generation, project, "message-delivery", thread, after] as const,
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
  taskHandoffs: (generation: number, project: string, task: string, cursor: string | null) =>
    ["live", generation, project, "task-handoffs", task, cursor] as const,
  taskSurface: (
    generation: number,
    project: string,
    task: string,
    kind: string,
    after: string | null,
  ) => ["live", generation, project, "task-surface", task, kind, after] as const,
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
  runContext: (generation: number, project: string, run: string) =>
    ["live", generation, project, "run-context", run] as const,
  runEvidence: (generation: number, project: string, run: string, cursor: string | null) =>
    ["live", generation, project, "run-evidence", run, cursor] as const,
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
