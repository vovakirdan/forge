import { z } from "zod";
import { UuidV7Schema, TimestampSchema } from "../contracts/common.ts";
import { ProjectViewSchema } from "../contracts/project.ts";

export const SessionSchema = z.object({
  token: z.string().regex(/^[0-9a-f]{64}$/),
  expires_at: TimestampSchema,
});
export type SessionCredentials = z.infer<typeof SessionSchema>;
const HealthSchema = z
  .object({ status: z.literal("ready"), api_version: z.string().min(1) })
  .passthrough();

export type ApiErrorKind =
  "unauthorized" | "not_found" | "http" | "network" | "invalid_response" | "invalid_id";
export class LiveApiError extends Error {
  readonly kind: ApiErrorKind;
  constructor(kind: ApiErrorKind) {
    super(kind);
    this.name = "LiveApiError";
    this.kind = kind;
  }
}

export function describeApiError(error: unknown): string {
  if (!(error instanceof LiveApiError)) return "The request could not be completed.";
  switch (error.kind) {
    case "unauthorized":
      return "The session is no longer authorized. Connect again.";
    case "not_found":
      return "Project not found.";
    case "http":
      return "Forge returned an error. Retry when the service is available.";
    case "network":
      return "Cannot reach Forge. Check the connection and retry.";
    case "invalid_response":
      return "Forge returned an unsupported response.";
    case "invalid_id":
      return "Enter a valid UUIDv7 Project ID.";
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

  return {
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
    async project(id: string, token: string, signal: AbortSignal) {
      if (!UuidV7Schema.safeParse(id).success) throw new LiveApiError("invalid_id");
      const value = await json(
        await request(`/api/projects/${id}`, "GET", signal, token),
        ProjectViewSchema,
      );
      if (value.id.toLowerCase() !== id.toLowerCase()) throw new LiveApiError("invalid_response");
      return value;
    },
  };
}
export type LiveApi = ReturnType<typeof createLiveApi>;
