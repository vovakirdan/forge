import { z } from "zod";
import { UuidV7Schema, TimestampSchema } from "../contracts/common.ts";
import { ProjectListResponseSchema, ProjectViewSchema } from "../contracts/project.ts";
import {
  EmployeeListResponseSchema,
  EmployeeProfileSchema,
  EmployeeRunListResponseSchema,
} from "../contracts/employee.ts";
import { PrioritySchemeViewSchema } from "../contracts/priority-scheme.ts";
import { TaskDetailViewSchema, TaskListResponseSchema } from "../contracts/task.ts";
import {
  PipelineVersionListResponseSchema,
  PipelineVersionViewSchema,
} from "../contracts/pipeline.ts";
import { RunDetailViewSchema, RunListResponseSchema } from "../contracts/run.ts";
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

  return {
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
  };
}
export type LiveApi = ReturnType<typeof createLiveApi>;
