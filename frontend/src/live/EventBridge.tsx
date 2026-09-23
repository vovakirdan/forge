import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { describeApiError } from "./api.ts";
import type { LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";
import type { ProjectEvent } from "./event-api.ts";
import type { LeaveGuard } from "./leave-guard.ts";

type Props = {
  api: LiveApi;
  session: LiveSession;
  generation: number;
  projectId: string;
  visible: boolean;
  leaveGuard: LeaveGuard;
};

export function EventBridge({ api, session, generation, projectId, visible, leaveGuard }: Props) {
  const queries = useQueryClient();
  const [events, setEvents] = useState<ProjectEvent[]>([]);
  const [status, setStatus] = useState<"connecting" | "current" | "stale">("connecting");
  const [error, setError] = useState<unknown>(null);
  const [restart, setRestart] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let after: number | null = null;
    let failures = 0;
    let caughtUp = false;
    let dirty = false;
    let polling = false;
    function schedule(delay: number) {
      clearTimeout(timer);
      if (navigator.onLine) timer = setTimeout(poll, delay);
    }
    async function poll() {
      if (polling) return;
      if (!navigator.onLine) {
        setStatus("stale");
        return;
      }
      polling = true;
      try {
        const batch = await session.request(generation, (token) =>
          api.events(projectId, after, token, controller.signal),
        );
        if (controller.signal.aborted) return;
        if (batch.length > 0) {
          after = batch.at(-1)!.project_sequence;
          setEvents((old) => [...old, ...batch].slice(-50));
          if (caughtUp) dirty = true;
        }
        if (batch.length < 1000) {
          if (dirty && !leaveGuard.hasPending()) {
            void queries.invalidateQueries({ queryKey: ["live", generation, projectId] });
            void queries.invalidateQueries({ queryKey: ["project", generation, projectId] });
            dirty = false;
          }
          caughtUp = true;
        }
        failures = 0;
        setError(null);
        setStatus(caughtUp ? "current" : "connecting");
        schedule(batch.length === 1000 ? 0 : 2_000);
      } catch (failure) {
        if (controller.signal.aborted) return;
        setError(failure);
        setStatus("stale");
        failures += 1;
        schedule(Math.min(30_000, 1_000 * 2 ** Math.min(failures, 5)));
      } finally {
        polling = false;
      }
    }
    function offline() {
      clearTimeout(timer);
      setStatus("stale");
    }
    function online() {
      if (!controller.signal.aborted && !polling) schedule(0);
    }
    window.addEventListener("offline", offline);
    window.addEventListener("online", online);
    void poll();
    return () => {
      controller.abort();
      clearTimeout(timer);
      window.removeEventListener("offline", offline);
      window.removeEventListener("online", online);
    };
  }, [api, session, generation, projectId, queries, restart, leaveGuard]);

  return (
    <section
      aria-label="Activity"
      className="space-y-3 rounded-xl border border-border bg-card p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">Activity</h2>
          <p role="status" className="text-xs text-muted-foreground">
            {status === "connecting"
              ? "Connecting to event history…"
              : status === "current"
                ? "Event history current"
                : "Event history stale; reconnecting…"}
          </p>
        </div>
        {status === "stale" && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setEvents([]);
              setRestart((value) => value + 1);
            }}
          >
            Reconnect
          </Button>
        )}
      </div>
      {status === "stale" && (
        <p role="alert" className="text-sm">
          {describeApiError(error)}
        </p>
      )}
      {visible &&
        (events.length === 0 ? (
          <p className="text-sm text-muted-foreground">No events loaded for this Project.</p>
        ) : (
          <ol className="space-y-2" aria-label="Recent events">
            {events
              .slice()
              .reverse()
              .map((event) => (
                <li
                  key={event.event_id}
                  className="flex flex-wrap gap-x-3 border-b border-border py-2 text-sm"
                >
                  <span className="font-mono text-xs text-muted-foreground">
                    #{event.project_sequence}
                  </span>
                  <span>{event.event_type}</span>
                  <time
                    className="ml-auto text-xs text-muted-foreground"
                    dateTime={event.occurred_at}
                  >
                    {new Date(event.occurred_at).toLocaleString()}
                  </time>
                </li>
              ))}
          </ol>
        ))}
    </section>
  );
}
