import { z } from "zod";
import { UuidV7Schema, TimestampSchema } from "../contracts/common.ts";
import { LiveApiError } from "./api.ts";

const EventSchema = z.object({
  schema_version: z.literal(1),
  event_id: UuidV7Schema,
  project_id: UuidV7Schema,
  project_sequence: z.number().int().safe().positive(),
  event_type: z.string().min(1),
  occurred_at: TimestampSchema,
});

export type ProjectEvent = z.infer<typeof EventSchema>;

export function parseEventBatch(
  body: string,
  projectId: string,
  after: number | null,
): ProjectEvent[] {
  const events: ProjectEvent[] = [];
  let id: string | null = null;
  let kind: string | null = null;
  let data: string | null = null;
  const lines = body.replaceAll("\r\n", "\n").split("\n");
  for (const line of [...lines, ""]) {
    if (line === "") {
      if (id !== null || kind !== null || data !== null) {
        if (id === null || kind !== "forge.event" || data === null)
          throw new LiveApiError("invalid_response");
        let value: unknown;
        try {
          value = JSON.parse(data);
        } catch {
          throw new LiveApiError("invalid_response");
        }
        const parsed = EventSchema.safeParse(value);
        if (
          !parsed.success ||
          parsed.data.project_id.toLowerCase() !== projectId.toLowerCase() ||
          String(parsed.data.project_sequence) !== id ||
          (after !== null && parsed.data.project_sequence <= after) ||
          (events.length > 0 && parsed.data.project_sequence <= events.at(-1)!.project_sequence)
        )
          throw new LiveApiError("invalid_response");
        events.push(parsed.data);
      }
      id = kind = data = null;
    } else if (line.startsWith(":")) {
      continue;
    } else if (line.startsWith("id: ") && id === null) {
      id = line.slice(4);
    } else if (line.startsWith("event: ") && kind === null) {
      kind = line.slice(7);
    } else if (line.startsWith("data: ") && data === null) {
      data = line.slice(6);
    } else {
      throw new LiveApiError("invalid_response");
    }
  }
  return events;
}

export async function readProjectEvents(
  fetcher: typeof fetch,
  projectId: string,
  after: number | null,
  token: string,
  signal: AbortSignal,
): Promise<ProjectEvent[]> {
  if (!UuidV7Schema.safeParse(projectId).success) throw new LiveApiError("invalid_id");
  const path = `/api/projects/${projectId}/events${after === null ? "" : `?after=${after}`}`;
  let response: Response;
  try {
    response = await fetcher(path, {
      method: "GET",
      signal,
      credentials: "omit",
      redirect: "error",
      cache: "no-store",
      headers: { Accept: "text/event-stream", Authorization: `Bearer ${token}` },
    });
  } catch {
    if (signal.aborted) throw new DOMException("Request cancelled", "AbortError");
    throw new LiveApiError("network");
  }
  if (!response.ok)
    throw new LiveApiError(
      response.status === 401 ? "unauthorized" : response.status === 404 ? "not_found" : "http",
    );
  if (response.headers.get("content-type")?.split(";")[0] !== "text/event-stream")
    throw new LiveApiError("invalid_response");
  const body = await response.text();
  if (body.length > 8 * 1024 * 1024) throw new LiveApiError("response_too_large");
  return parseEventBatch(body, projectId, after);
}
