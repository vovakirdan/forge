import { z } from "zod";
import { readProjectEvents } from "./event-api.ts";
import { UuidV7Schema, StableKeySchema, TimestampSchema } from "../contracts/common.ts";
import { ProjectListResponseSchema, ProjectViewSchema } from "../contracts/project.ts";
import { ProjectResourcesSchema } from "../contracts/resources.ts";
import { ProjectTaskPropertySchemaSchema } from "../contracts/task-property-schema.ts";
import {
  EmployeeListResponseSchema,
  EmployeeProfileSchema,
  EmployeeRunListResponseSchema,
  EmployeeOperationsSchema,
} from "../contracts/employee.ts";
import { PrioritySchemeViewSchema } from "../contracts/priority-scheme.ts";
import {
  DerivedMemoryEntrySchema,
  DerivedMemoryHistorySchema,
  DerivedMemoryListSchema,
  EmployeeOnboardingStatusSchema,
  KnowledgeHistorySchema,
  KnowledgePageListSchema,
  KnowledgePageSchema,
  MemoryProjectionStatusSchema,
  MemorySearchResultSchema,
} from "../contracts/knowledge.ts";
import { TaskDetailViewSchema, TaskListResponseSchema } from "../contracts/task.ts";
import { TaskHandoffPageSchema } from "../contracts/task-handoff.ts";
import { EmployeeRuntimeMetadataSchema } from "../contracts/employee-runtime-metadata.ts";
import {
  PipelineCatalogListSchema,
  PipelineVersionListResponseSchema,
  PipelineVersionViewSchema,
} from "../contracts/pipeline.ts";
import { RunDetailViewSchema, RunListResponseSchema } from "../contracts/run.ts";
import {
  RunContextCoordinatesSchema,
  RunEvidencePageSchema,
} from "../contracts/run-context-evidence.ts";
import { SystemJobAttemptPageSchema, SystemJobStatusSchema } from "../contracts/system-jobs.ts";
import {
  ProjectHookInvocationListSchema,
  ProjectHookListSchema,
} from "../contracts/project-hook.ts";
import {
  EmployeeMessageListSchema,
  EmployeeThreadListSchema,
  MessageDeliveryListSchema,
} from "../contracts/communication.ts";
import type { InboxAttempt } from "../contracts/communication-command.ts";
import { sendInboxCommand } from "./inbox-command-api.ts";
import {
  ResumeScheduleListSchema,
  EscalationListSchema,
  EscalationSchema,
  ResolverRouteListSchema,
  NextRunConstraintListSchema,
  RecoveryRunListSchema,
  RecoverySettingsSchema,
  RecoveryReadinessSchema,
} from "../contracts/management.ts";
import type { ManagementAttempt } from "../contracts/management-command.ts";
import { sendManagementCommand } from "./management-command-api.ts";
import {
  GitSourceSettingSchema,
  FileSnapshotsSchema,
  FileInputsSchema,
  CandidateReviewsSchema,
  GitIntegrationsSchema,
} from "../contracts/surface-artifact.ts";
import { sendAmendDraft } from "./amend-draft-api.ts";
import type { DraftAttempt } from "./draft-attempt.ts";
import { sendSetTaskPriority } from "./set-task-priority-api.ts";
import type { TaskCommandAttempt } from "../contracts/task-command.ts";
import type { CreateTaskAttempt } from "../contracts/create-task.ts";
import { sendCreateTask } from "./create-task-api.ts";
import { sendCreateEmployee } from "./create-employee-api.ts";
import type { CreateEmployeeAttempt } from "../contracts/create-employee.ts";
import { sendAmendEmployee } from "./amend-employee-api.ts";
import type { AmendEmployeeAttempt } from "../contracts/amend-employee.ts";
import { sendEmployeeLifecycle } from "./employee-lifecycle-api.ts";
import type { EmployeeLifecycleAttempt } from "../contracts/employee-lifecycle.ts";
import { sendApproveTask } from "./approve-task-api.ts";
import { sendCancelTask } from "./cancel-task-api.ts";
import { sendDependencyCommand } from "./dependency-command-api.ts";
import type { DependencyCommandAttempt } from "../contracts/dependency-command.ts";
import { CancellationReasonsViewSchema } from "../contracts/cancellation-reasons.ts";
import {
  DependencyDirectionSchema,
  TaskDependencyListResponseSchema,
  type DependencyDirection,
} from "../contracts/task-dependencies.ts";

export const SessionSchema = z.object({
  token: z.string().regex(/^[0-9a-f]{64}$/),
  expires_at: TimestampSchema,
});
export type SessionCredentials = z.infer<typeof SessionSchema>;
const HealthSchema = z
  .object({ status: z.literal("ready"), api_version: z.string().min(1) })
  .passthrough();

export type ApiErrorKind =
  | "unauthorized"
  | "not_found"
  | "http"
  | "network"
  | "invalid_response"
  | "invalid_id"
  | "response_too_large"
  | "cursor_invalid";
export class LiveApiError extends Error {
  readonly kind: ApiErrorKind;
  constructor(kind: ApiErrorKind) {
    super(kind);
    this.name = "LiveApiError";
    this.kind = kind;
  }
}

export function describeApiError(
  error: unknown,
  resource: "Project" | "Task" | "Pipeline" | "Run" | "Employee" = "Project",
): string {
  if (!(error instanceof LiveApiError)) return "The request could not be completed.";
  switch (error.kind) {
    case "unauthorized":
      return "The session is no longer authorized. Connect again.";
    case "not_found":
      return `${resource} not found.`;
    case "http":
      return "Forge returned an error. Retry when the service is available.";
    case "network":
      return "Cannot reach Forge. Check the connection and retry.";
    case "invalid_response":
      return "Forge returned an unsupported response.";
    case "invalid_id":
      return `Enter a valid UUIDv7 ${resource} ID.`;
    case "response_too_large":
      return "This response exceeds the read-size limit. Its contents were not loaded.";
    case "cursor_invalid":
      return "This page cursor is no longer valid. Restart pagination to read the current list.";
  }
}

export function createLiveApi(fetcher: typeof fetch = fetch) {
  async function request(
    path: string,
    method: "GET" | "POST",
    signal: AbortSignal,
    token?: string,
    body?: { code: string },
  ) {
    let response: Response;
    try {
      response = await fetcher(path, {
        method,
        signal,
        credentials: "omit",
        redirect: "error",
        cache: "no-store",
        headers: {
          Accept: "application/json",
          ...(token === undefined ? {} : { Authorization: `Bearer ${token}` }),
          ...(body === undefined ? {} : { "Content-Type": "application/json" }),
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
    } catch {
      if (signal.aborted) throw new DOMException("Request cancelled", "AbortError");
      throw new LiveApiError("network");
    }
    if (!response.ok) {
      if (response.status === 409 || response.status === 502) {
        const envelope: unknown = await response.json().catch(() => null);
        const parsed = z.object({ error: z.string(), code: z.string() }).safeParse(envelope);
        if (parsed.success) {
          if (response.status === 409 && parsed.data.code === "cursor_invalid")
            throw new LiveApiError("cursor_invalid");
          if (response.status === 502 && parsed.data.code === "response_too_large")
            throw new LiveApiError("response_too_large");
        }
      }
      throw new LiveApiError(
        response.status === 401 ? "unauthorized" : response.status === 404 ? "not_found" : "http",
      );
    }
    return response;
  }

  async function json<T>(
    response: Response,
    schema: z.ZodType<T, z.ZodTypeDef, unknown>,
  ): Promise<T> {
    try {
      const value: unknown = await response.json();
      const result = schema.safeParse(value);
      if (result.success) return result.data;
    } catch {
      /* Never expose response bodies or validation details to the UI. */
    }
    throw new LiveApiError("invalid_response");
  }

  function identifiers(...values: string[]) {
    if (values.some((value) => !UuidV7Schema.safeParse(value).success))
      throw new LiveApiError("invalid_id");
  }

  function revisionCursor(value: string | null) {
    if (
      value !== null &&
      (value.length > 19 || !/^\d+$/.test(value) || BigInt(value) > 9_223_372_036_854_775_807n)
    )
      throw new LiveApiError("cursor_invalid");
  }

  function scopedPage<T extends { items: { project_id: string; id: string }[] }>(
    page: T,
    projectId: string,
    uniqueIds = true,
  ): T {
    if (
      page.items.length > 20 ||
      page.items.some((item) => item.project_id.toLowerCase() !== projectId.toLowerCase()) ||
      (uniqueIds &&
        new Set(page.items.map((item) => item.id.toLowerCase())).size !== page.items.length)
    )
      throw new LiveApiError("invalid_response");
    return page;
  }

  return {
    async recoveryAssessmentReadiness(
      projectId: string,
      runId: string,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, runId);
      const value = await json(
        await request(
          `/api/projects/${projectId}/recovery-runs/${runId}/assessment-readiness`,
          "GET",
          signal,
          token,
        ),
        RecoveryReadinessSchema,
      );
      if (value.run_id.toLowerCase() !== runId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async managementFactsPage(
      projectId: string,
      kind: "resolver-routes" | "next-run-constraints" | "recovery-runs",
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (cursor !== null) {
        if (kind === "resolver-routes") StableKeySchema.parse(cursor);
        else identifiers(cursor);
      }
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const response = await request(
        `/api/projects/${projectId}/${kind}?${search}`,
        "GET",
        signal,
        token,
      );
      const validate = (
        page: { items: unknown[]; next_cursor?: string | undefined },
        ids: string[],
      ) => {
        if (page.next_cursor !== undefined && page.next_cursor !== ids.at(-1))
          throw new LiveApiError("invalid_response");
        if (
          new Set(ids).size !== ids.length ||
          ids.some((id, index) => index > 0 && id <= ids[index - 1]!)
        )
          throw new LiveApiError("invalid_response");
      };
      if (kind === "resolver-routes") {
        const value = await json(response, ResolverRouteListSchema);
        validate(
          value,
          value.items.map((item) => item.key),
        );
        return { kind, value } as const;
      }
      if (kind === "next-run-constraints") {
        const value = await json(response, NextRunConstraintListSchema);
        validate(
          value,
          value.items.map((item) => item.id),
        );
        return { kind, value } as const;
      }
      const value = await json(response, RecoveryRunListSchema);
      validate(
        value,
        value.items.map((item) => item.run_id),
      );
      return { kind, value } as const;
    },
    async recoverySettings(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      return json(
        await request(`/api/projects/${projectId}/recovery`, "GET", signal, token),
        RecoverySettingsSchema,
      );
    },
    managementCommand(attempt: ManagementAttempt, token: string, signal: AbortSignal) {
      return sendManagementCommand(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    async managementPage(
      projectId: string,
      kind: "resume-schedules" | "escalations",
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (cursor !== null) identifiers(cursor);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const response = await request(
        `/api/projects/${projectId}/${kind}?${search}`,
        "GET",
        signal,
        token,
      );
      function valid(page: { items: { id: string }[]; next_cursor?: string | undefined }) {
        if (page.next_cursor !== undefined && page.next_cursor !== page.items.at(-1)?.id)
          throw new LiveApiError("invalid_response");
        if (
          new Set(page.items.map((item) => item.id)).size !== page.items.length ||
          page.items.some((item, index) => index > 0 && item.id <= page.items[index - 1]!.id)
        )
          throw new LiveApiError("invalid_response");
      }
      if (kind === "escalations") {
        const value = await json(response, EscalationListSchema);
        valid(value);
        return { kind, value } as const;
      }
      const value = await json(response, ResumeScheduleListSchema);
      valid(value);
      return { kind, value } as const;
    },
    async escalationDetail(
      projectId: string,
      escalationId: string,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, escalationId);
      const value = await json(
        await request(
          `/api/projects/${projectId}/escalations/${escalationId}`,
          "GET",
          signal,
          token,
        ),
        EscalationSchema.passthrough(),
      );
      if (value.id.toLowerCase() !== escalationId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    inboxCommand(attempt: InboxAttempt, token: string, signal: AbortSignal) {
      return sendInboxCommand(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    events(projectId: string, after: number | null, token: string, signal: AbortSignal) {
      return readProjectEvents(fetcher, projectId, after, token, signal);
    },
    amendEmployee(attempt: AmendEmployeeAttempt, token: string, signal: AbortSignal) {
      return sendAmendEmployee(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    employeeLifecycle(attempt: EmployeeLifecycleAttempt, token: string, signal: AbortSignal) {
      return sendEmployeeLifecycle(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    createEmployee(attempt: CreateEmployeeAttempt, token: string, signal: AbortSignal) {
      return sendCreateEmployee(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    dependencyCommand(attempt: DependencyCommandAttempt, token: string, signal: AbortSignal) {
      return sendDependencyCommand(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    async taskDependencies(
      projectId: string,
      taskId: string,
      direction: DependencyDirection,
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, taskId);
      if (!DependencyDirectionSchema.safeParse(direction).success)
        throw new LiveApiError("invalid_id");
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(
          `/api/projects/${projectId}/tasks/${taskId}/dependencies/${direction}?${search}`,
          "GET",
          signal,
          token,
        ),
        TaskDependencyListResponseSchema,
      );
      const relatedIds = new Set<string>();
      if (
        value.items.length > 20 ||
        value.items.some((item) => {
          const selected = direction === "blocked_by" ? item.blocked_task_id : item.blocker_task_id;
          const related = direction === "blocked_by" ? item.blocker_task_id : item.blocked_task_id;
          const relatedId = related.toLowerCase();
          const invalid =
            selected.toLowerCase() !== taskId.toLowerCase() ||
            relatedId !== item.related_task.id.toLowerCase() ||
            relatedId === taskId.toLowerCase() ||
            relatedIds.has(relatedId);
          relatedIds.add(relatedId);
          return invalid;
        })
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    cancelTask(attempt: TaskCommandAttempt, token: string, signal: AbortSignal) {
      return sendCancelTask(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    async cancellationReasons(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const value = await json(
        await request(`/api/projects/${projectId}/cancellation-reasons`, "GET", signal, token),
        CancellationReasonsViewSchema,
      );
      if (value.project_id.toLowerCase() !== projectId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    approveTask(attempt: TaskCommandAttempt, token: string, signal: AbortSignal) {
      return sendApproveTask(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    createTask(attempt: CreateTaskAttempt, token: string, signal: AbortSignal) {
      return sendCreateTask(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    setTaskPriority(attempt: TaskCommandAttempt, token: string, signal: AbortSignal) {
      return sendSetTaskPriority(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    amendDraft(attempt: DraftAttempt, token: string, signal: AbortSignal) {
      return sendAmendDraft(
        fetcher,
        attempt,
        token,
        signal,
        () => new LiveApiError("unauthorized"),
      );
    },
    async exchange(code: string, signal: AbortSignal) {
      return json(
        await request("/api/auth/exchange", "POST", signal, undefined, { code }),
        SessionSchema,
      );
    },
    async logout(token: string, signal: AbortSignal) {
      const response = await request("/api/auth/logout", "POST", signal, token);
      if (response.status !== 204) throw new LiveApiError("invalid_response");
    },
    async health(token: string, signal: AbortSignal) {
      return json(await request("/api/health", "GET", signal, token), HealthSchema);
    },
    async projects(cursor: string | null, token: string, signal: AbortSignal) {
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(`/api/projects?${search}`, "GET", signal, token),
        ProjectListResponseSchema,
      );
      if (
        value.items.length > 20 ||
        new Set(value.items.map((item) => item.id)).size !== value.items.length
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async project(id: string, token: string, signal: AbortSignal) {
      identifiers(id);
      const value = await json(
        await request(`/api/projects/${id}`, "GET", signal, token),
        ProjectViewSchema,
      );
      if (value.id.toLowerCase() !== id.toLowerCase()) throw new LiveApiError("invalid_response");
      return value;
    },
    async resources(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const value = await json(
        await request(`/api/projects/${projectId}/resources`, "GET", signal, token),
        ProjectResourcesSchema,
      );
      if (value.project_id.toLowerCase() !== projectId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async taskPropertySchema(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const value = await json(
        await request(`/api/projects/${projectId}/task-property-schema`, "GET", signal, token),
        ProjectTaskPropertySchemaSchema,
      );
      if (value.project_id.toLowerCase() !== projectId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async pipelineCatalog(
      projectId: string,
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (cursor !== null) identifiers(cursor);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(
          `/api/projects/${projectId}/pipeline-catalog?${search}`,
          "GET",
          signal,
          token,
        ),
        PipelineCatalogListSchema,
      );
      if (new Set(value.items.map((item) => item.id)).size !== value.items.length)
        throw new LiveApiError("invalid_response");
      return value;
    },
    async projectHooks(
      projectId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (after !== null) identifiers(after);
      const query = new URLSearchParams({ limit: "20" });
      if (after !== null) query.set("after", after);
      const value = await json(
        await request(`/api/projects/${projectId}/hook-versions?${query}`, "GET", signal, token),
        ProjectHookListSchema,
      );
      if (
        value.items.some((item) => item.project_id.toLowerCase() !== projectId.toLowerCase()) ||
        new Set(value.items.map((item) => item.id)).size !== value.items.length
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async projectHookInvocations(
      projectId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (after !== null) identifiers(after);
      const query = new URLSearchParams({ limit: "20" });
      if (after !== null) query.set("after", after);
      const value = await json(
        await request(`/api/projects/${projectId}/hook-invocations?${query}`, "GET", signal, token),
        ProjectHookInvocationListSchema,
      );
      if (new Set(value.items.map((item) => item.id)).size !== value.items.length)
        throw new LiveApiError("invalid_response");
      return value;
    },
    async knowledgePages(
      projectId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (after !== null) identifiers(after);
      const query = new URLSearchParams({ limit: "20" });
      if (after !== null) query.set("after", after);
      return scopedPage(
        await json(
          await request(`/api/projects/${projectId}/knowledge?${query}`, "GET", signal, token),
          KnowledgePageListSchema,
        ),
        projectId,
      );
    },
    async knowledgePage(projectId: string, pageId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, pageId);
      const value = await json(
        await request(`/api/projects/${projectId}/knowledge/${pageId}`, "GET", signal, token),
        KnowledgePageSchema,
      );
      if (
        value.project_id.toLowerCase() !== projectId.toLowerCase() ||
        value.id.toLowerCase() !== pageId.toLowerCase()
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async knowledgeHistory(
      projectId: string,
      pageId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, pageId);
      revisionCursor(after);
      const query = new URLSearchParams({ limit: "20" });
      if (after !== null) query.set("after", after);
      const value = scopedPage(
        await json(
          await request(
            `/api/projects/${projectId}/knowledge/${pageId}/history?${query}`,
            "GET",
            signal,
            token,
          ),
          KnowledgeHistorySchema,
        ),
        projectId,
        false,
      );
      if (value.items.some((item) => item.id.toLowerCase() !== pageId.toLowerCase()))
        throw new LiveApiError("invalid_response");
      return value;
    },
    async memoryEntries(
      projectId: string,
      employeeId: string | null,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (employeeId !== null) identifiers(employeeId);
      if (after !== null) identifiers(after);
      const query = new URLSearchParams({ limit: "20" });
      if (employeeId !== null) query.set("employee_id", employeeId);
      if (after !== null) query.set("after", after);
      const value = scopedPage(
        await json(
          await request(`/api/projects/${projectId}/memory?${query}`, "GET", signal, token),
          DerivedMemoryListSchema,
        ),
        projectId,
      );
      if (
        value.items.some(
          (item) =>
            item.withdrawn ||
            (item.subject.kind === "employee_memory_entry" &&
              (employeeId === null ||
                item.subject.employee_id.toLowerCase() !== employeeId.toLowerCase())),
        )
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async memoryEntry(projectId: string, entryId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, entryId);
      const value = await json(
        await request(`/api/projects/${projectId}/memory/${entryId}`, "GET", signal, token),
        DerivedMemoryEntrySchema,
      );
      if (
        value.project_id.toLowerCase() !== projectId.toLowerCase() ||
        value.id.toLowerCase() !== entryId.toLowerCase()
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async memoryHistory(
      projectId: string,
      entryId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, entryId);
      revisionCursor(after);
      const query = new URLSearchParams({ limit: "20" });
      if (after !== null) query.set("after", after);
      const value = scopedPage(
        await json(
          await request(
            `/api/projects/${projectId}/memory/${entryId}/history?${query}`,
            "GET",
            signal,
            token,
          ),
          DerivedMemoryHistorySchema,
        ),
        projectId,
        false,
      );
      if (value.items.some((item) => item.id.toLowerCase() !== entryId.toLowerCase()))
        throw new LiveApiError("invalid_response");
      return value;
    },
    async memorySearch(
      projectId: string,
      employeeId: string | null,
      term: string,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId);
      if (employeeId !== null) identifiers(employeeId);
      if (!term.trim() || new TextEncoder().encode(term).length > 8192)
        throw new LiveApiError("invalid_id");
      const query = new URLSearchParams({ limit: "20", query: term });
      if (employeeId !== null) query.set("employee_id", employeeId);
      const value = await json(
        await request(`/api/projects/${projectId}/memory/search?${query}`, "GET", signal, token),
        MemorySearchResultSchema,
      );
      if (
        value.project_id.toLowerCase() !== projectId.toLowerCase() ||
        value.employee_id?.toLowerCase() !== employeeId?.toLowerCase() ||
        value.results.length > 20 ||
        value.results.some((hit) => {
          const document = hit.document;
          if (document.record.project_id.toLowerCase() !== projectId.toLowerCase()) return true;
          if (document.kind === "knowledge_page") return document.record.status !== "published";
          const entry = document.record;
          return (
            entry.withdrawn ||
            (entry.subject.kind === "employee_memory_entry" &&
              (employeeId === null ||
                entry.subject.employee_id.toLowerCase() !== employeeId.toLowerCase()))
          );
        })
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async memoryStatus(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      return json(
        await request(`/api/projects/${projectId}/memory/status`, "GET", signal, token),
        MemoryProjectionStatusSchema,
      );
    },
    async systemJobs(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const value = await json(
        await request(`/api/projects/${projectId}/system-jobs`, "GET", signal, token),
        SystemJobStatusSchema,
      );
      if (
        new Set(value.jobs.map((job) => job.id)).size !== value.jobs.length ||
        value.jobs.some((job) => job.project_id.toLowerCase() !== projectId.toLowerCase()) ||
        new Set(value.onboarding.map((item) => item.employee_id)).size !== value.onboarding.length
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async systemJobAttempts(
      projectId: string,
      jobId: string,
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, jobId);
      if (cursor !== null) identifiers(cursor);
      const query = new URLSearchParams({ limit: "20" });
      if (cursor !== null) query.set("cursor", cursor);
      const value = await json(
        await request(
          `/api/projects/${projectId}/system-jobs/${jobId}/attempts?${query}`,
          "GET",
          signal,
          token,
        ),
        SystemJobAttemptPageSchema,
      );
      if (
        value.items.some((item) => item.job_id.toLowerCase() !== jobId.toLowerCase()) ||
        new Set(value.items.map((item) => item.id)).size !== value.items.length
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employeeOnboarding(
      projectId: string,
      employeeId: string,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, employeeId);
      const value = await json(
        await request(
          `/api/projects/${projectId}/employees/${employeeId}/onboarding`,
          "GET",
          signal,
          token,
        ),
        EmployeeOnboardingStatusSchema,
      );
      if (value.employee_id.toLowerCase() !== employeeId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employees(projectId: string, cursor: string | null, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(`/api/projects/${projectId}/employees?${search}`, "GET", signal, token),
        EmployeeListResponseSchema,
      );
      if (
        value.items.length > 20 ||
        new Set(value.items.map((item) => item.id)).size !== value.items.length
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employee(projectId: string, employeeId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, employeeId);
      const value = await json(
        await request(`/api/projects/${projectId}/employees/${employeeId}`, "GET", signal, token),
        EmployeeProfileSchema,
      );
      if (
        value.id.toLowerCase() !== employeeId.toLowerCase() ||
        value.project_id.toLowerCase() !== projectId.toLowerCase()
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employeeOperations(
      projectId: string,
      employeeId: string,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, employeeId);
      const value = await json(
        await request(
          `/api/projects/${projectId}/employees/${employeeId}/operations`,
          "GET",
          signal,
          token,
        ),
        EmployeeOperationsSchema,
      );
      if (value.employee_id.toLowerCase() !== employeeId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employeeRuntimeMetadata(
      projectId: string,
      employeeId: string,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, employeeId);
      const value = await json(
        await request(
          `/api/projects/${projectId}/employees/${employeeId}/runtime-metadata`,
          "GET",
          signal,
          token,
        ),
        EmployeeRuntimeMetadataSchema,
      );
      if (value.employee_id.toLowerCase() !== employeeId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employeeRuns(
      projectId: string,
      employeeId: string,
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, employeeId);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(
          `/api/projects/${projectId}/employees/${employeeId}/runs?${search}`,
          "GET",
          signal,
          token,
        ),
        EmployeeRunListResponseSchema,
      );
      if (
        value.items.length > 20 ||
        new Set(value.items.map((item) => item.id)).size !== value.items.length ||
        value.items.some((item) => item.employee_id?.toLowerCase() !== employeeId.toLowerCase())
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employeeThreads(
      projectId: string,
      employeeId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, employeeId);
      if (after !== null) identifiers(after);
      const search = new URLSearchParams({ limit: "20" });
      if (after !== null) search.set("after", after);
      const value = await json(
        await request(
          `/api/projects/${projectId}/employees/${employeeId}/threads?${search}`,
          "GET",
          signal,
          token,
        ),
        EmployeeThreadListSchema,
      );
      if (
        new Set(value.items.map((item) => item.id)).size !== value.items.length ||
        (value.next_cursor !== undefined && value.next_cursor !== value.items.at(-1)?.id) ||
        value.items.some(
          (item) =>
            item.project_id.toLowerCase() !== projectId.toLowerCase() ||
            item.employee_id.toLowerCase() !== employeeId.toLowerCase(),
        )
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async employeeMessages(
      projectId: string,
      employeeId: string,
      threadId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, employeeId, threadId);
      if (
        after !== null &&
        (!/^(0|[1-9][0-9]*)$/.test(after) || BigInt(after) > BigInt(Number.MAX_SAFE_INTEGER))
      )
        throw new LiveApiError("cursor_invalid");
      const search = new URLSearchParams({ limit: "20" });
      if (after !== null) search.set("after", after);
      const value = await json(
        await request(
          `/api/projects/${projectId}/threads/${threadId}/messages?${search}`,
          "GET",
          signal,
          token,
        ),
        EmployeeMessageListSchema,
      );
      if (
        new Set(value.items.map((item) => item.id)).size !== value.items.length ||
        (value.next_cursor !== undefined &&
          value.next_cursor !== String(value.items.at(-1)?.sequence)) ||
        value.items.some(
          (item) =>
            item.project_id.toLowerCase() !== projectId.toLowerCase() ||
            item.employee_id.toLowerCase() !== employeeId.toLowerCase() ||
            item.thread_id.toLowerCase() !== threadId.toLowerCase(),
        ) ||
        value.items.some(
          (item, index) =>
            (index > 0 && item.sequence <= value.items[index - 1]!.sequence) ||
            (after !== null && item.sequence <= Number(after)),
        )
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async messageDelivery(
      projectId: string,
      threadId: string,
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, threadId);
      if (
        after !== null &&
        (!/^(0|[1-9][0-9]*)$/.test(after) || BigInt(after) > BigInt(Number.MAX_SAFE_INTEGER))
      )
        throw new LiveApiError("cursor_invalid");
      const search = new URLSearchParams({ limit: "20" });
      if (after !== null) search.set("after", after);
      const value = await json(
        await request(
          `/api/projects/${projectId}/threads/${threadId}/delivery?${search}`,
          "GET",
          signal,
          token,
        ),
        MessageDeliveryListSchema,
      );
      if (
        value.next_cursor !== undefined &&
        value.next_cursor !== String(value.items.at(-1)?.sequence)
      )
        throw new LiveApiError("invalid_response");
      if (
        new Set(value.items.map((item) => item.message_id)).size !== value.items.length ||
        value.items.some(
          (item, index) =>
            (index > 0 && item.sequence <= value.items[index - 1]!.sequence) ||
            (after !== null && item.sequence <= Number(after)) ||
            (item.waiver !== null &&
              (item.waiver.message_id.toLowerCase() !== item.message_id.toLowerCase() ||
                item.waiver.project_id.toLowerCase() !== projectId.toLowerCase())),
        )
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async tasks(projectId: string, cursor: string | null, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(`/api/projects/${projectId}/tasks?${search}`, "GET", signal, token),
        TaskListResponseSchema,
      );
      if (value.items.length > 20) throw new LiveApiError("invalid_response");
      return value;
    },
    async priorityScheme(projectId: string, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const value = await json(
        await request(`/api/projects/${projectId}/priority-scheme`, "GET", signal, token),
        PrioritySchemeViewSchema,
      );
      if (value.project_id.toLowerCase() !== projectId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async task(projectId: string, taskId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, taskId);
      const value = await json(
        await request(`/api/projects/${projectId}/tasks/${taskId}`, "GET", signal, token),
        TaskDetailViewSchema,
      );
      if (value.id.toLowerCase() !== taskId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async taskHandoffs(
      projectId: string,
      taskId: string,
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, taskId);
      if (cursor !== null) identifiers(cursor);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(
          `/api/projects/${projectId}/tasks/${taskId}/handoffs?${search}`,
          "GET",
          signal,
          token,
        ),
        TaskHandoffPageSchema,
      );
      if (
        value.items.some(
          (item) =>
            item.project_id.toLowerCase() !== projectId.toLowerCase() ||
            item.task_id.toLowerCase() !== taskId.toLowerCase(),
        ) ||
        (value.next_cursor !== undefined && value.next_cursor !== value.items.at(-1)?.id)
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async taskSurface(
      projectId: string,
      taskId: string,
      kind: "git-source-policy" | "file-inputs" | "file-snapshots" | "reviews" | "integrations",
      after: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, taskId);
      if (after !== null) identifiers(after);
      const paged = kind === "file-snapshots" || kind === "reviews" || kind === "integrations";
      if (!paged && after !== null) throw new LiveApiError("cursor_invalid");
      const query = paged ? `?limit=20${after === null ? "" : `&after=${after}`}` : "";
      const response = await request(
        `/api/projects/${projectId}/tasks/${taskId}/${kind}${query}`,
        "GET",
        signal,
        token,
      );
      if (kind === "git-source-policy")
        return { kind, value: await json(response, GitSourceSettingSchema) } as const;
      if (kind === "file-inputs") {
        const value = await json(response, FileInputsSchema);
        if (
          value.items.some(
            (item) => item.manifest.project_id.toLowerCase() !== projectId.toLowerCase(),
          )
        )
          throw new LiveApiError("invalid_response");
        return { kind, value } as const;
      }
      if (kind === "file-snapshots") {
        const value = await json(response, FileSnapshotsSchema);
        if (
          value.items.some(
            (item) =>
              item.task_id.toLowerCase() !== taskId.toLowerCase() ||
              (item.manifest !== null &&
                (item.manifest.project_id.toLowerCase() !== projectId.toLowerCase() ||
                  item.manifest.source_task_id.toLowerCase() !== taskId.toLowerCase())),
          )
        )
          throw new LiveApiError("invalid_response");
        return { kind, value } as const;
      }
      if (kind === "reviews") {
        const value = await json(response, CandidateReviewsSchema);
        if (
          value.items.some(
            (item) =>
              item.project_id.toLowerCase() !== projectId.toLowerCase() ||
              item.task_id.toLowerCase() !== taskId.toLowerCase(),
          )
        )
          throw new LiveApiError("invalid_response");
        return { kind, value } as const;
      }
      const value = await json(response, GitIntegrationsSchema);
      if (value.items.some((item) => item.task_id.toLowerCase() !== taskId.toLowerCase()))
        throw new LiveApiError("invalid_response");
      return { kind, value } as const;
    },
    async pipelines(projectId: string, cursor: string | null, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(`/api/projects/${projectId}/pipelines?${search}`, "GET", signal, token),
        PipelineVersionListResponseSchema,
      );
      if (value.items.length > 20) throw new LiveApiError("invalid_response");
      return value;
    },
    async pipeline(projectId: string, versionId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, versionId);
      const value = await json(
        await request(`/api/projects/${projectId}/pipelines/${versionId}`, "GET", signal, token),
        PipelineVersionViewSchema,
      );
      if (value.id.toLowerCase() !== versionId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async runs(projectId: string, cursor: string | null, token: string, signal: AbortSignal) {
      identifiers(projectId);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(`/api/projects/${projectId}/runs?${search}`, "GET", signal, token),
        RunListResponseSchema,
      );
      if (value.items.length > 20) throw new LiveApiError("invalid_response");
      return value;
    },
    async run(projectId: string, runId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, runId);
      const value = await json(
        await request(`/api/projects/${projectId}/runs/${runId}`, "GET", signal, token),
        RunDetailViewSchema,
      );
      if (value.id.toLowerCase() !== runId.toLowerCase())
        throw new LiveApiError("invalid_response");
      return value;
    },
    async runContext(projectId: string, runId: string, token: string, signal: AbortSignal) {
      identifiers(projectId, runId);
      const value = await json(
        await request(`/api/projects/${projectId}/runs/${runId}/context`, "GET", signal, token),
        RunContextCoordinatesSchema,
      );
      if (
        value.availability === "available"
          ? value.coordinates.run_id.toLowerCase() !== runId.toLowerCase() ||
            value.coordinates.project_id.toLowerCase() !== projectId.toLowerCase()
          : value.run_id.toLowerCase() !== runId.toLowerCase()
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
    async runEvidence(
      projectId: string,
      runId: string,
      cursor: string | null,
      token: string,
      signal: AbortSignal,
    ) {
      identifiers(projectId, runId);
      if (cursor !== null) identifiers(cursor);
      const search = new URLSearchParams({ limit: "20" });
      if (cursor !== null) search.set("cursor", cursor);
      const value = await json(
        await request(
          `/api/projects/${projectId}/runs/${runId}/evidence?${search}`,
          "GET",
          signal,
          token,
        ),
        RunEvidencePageSchema,
      );
      if (
        value.items.length > 20 ||
        value.items.some((item) => item.run_id.toLowerCase() !== runId.toLowerCase())
      )
        throw new LiveApiError("invalid_response");
      return value;
    },
  };
}
export type LiveApi = ReturnType<typeof createLiveApi>;
