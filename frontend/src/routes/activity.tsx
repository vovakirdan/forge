import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ActivityService, AgentService } from "@/services";
import type { ActivityEvent } from "@/data/types";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, Initials, Panel } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { clockTime, relTime } from "@/lib/format";

export const Route = createFileRoute("/activity")({
  head: () => ({
    meta: [
      { title: "Activity — Forge" },
      {
        name: "description",
        content:
          "Chronological event stream of tasks, runs, agents, reviews, verification and decisions.",
      },
      { property: "og:title", content: "Activity — Forge" },
      { property: "og:description", content: "Everything the AI engineering team did, in order." },
    ],
  }),
  component: ActivityPage,
});

const CATEGORIES: (ActivityEvent["category"] | "all")[] = [
  "all",
  "tasks",
  "runs",
  "agents",
  "reviews",
  "verification",
  "decisions",
  "system",
];

const tone: Record<
  string,
  "primary" | "running" | "info" | "warning" | "success" | "danger" | "muted"
> = {
  tasks: "primary",
  runs: "running",
  agents: "info",
  reviews: "warning",
  verification: "success",
  decisions: "primary",
  system: "muted",
};

function ActivityPage() {
  const [category, setCategory] = useState<(typeof CATEGORIES)[number]>("all");
  const { data: events } = useQuery({
    queryKey: ["activity", category],
    queryFn: () => ActivityService.list(category === "all" ? undefined : category),
  });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });

  return (
    <Page>
      <PageHeader
        title="Activity"
        subtitle="Every state transition the pipeline produced, newest first"
        tabs={
          <div className="flex flex-wrap gap-1">
            {CATEGORIES.map((c) => (
              <Button
                key={c}
                size="sm"
                variant={category === c ? "secondary" : "ghost"}
                className="h-6 text-[11.5px] capitalize"
                onClick={() => setCategory(c)}
              >
                {c}
              </Button>
            ))}
          </div>
        }
      />
      <PageBody>
        <Panel dense>
          {!events ? (
            <div className="space-y-2 p-3">
              {Array.from({ length: 8 }).map((_, i) => (
                <Skeleton key={i} className="h-8 w-full" />
              ))}
            </div>
          ) : (
            <ol className="divide-y divide-border">
              {events.map((e) => {
                const actor = agents?.find((a) => a.id === e.actorId);
                return (
                  <li
                    key={e.id}
                    className="grid grid-cols-[54px_84px_1fr_auto] items-center gap-3 px-3 py-2"
                  >
                    <span className="mono-xs tabular-nums text-muted-foreground">
                      {clockTime(e.at)}
                    </span>
                    <Chip tone={tone[e.category]}>
                      <Dot tone={tone[e.category]} />
                      {e.category}
                    </Chip>
                    <div className="flex min-w-0 items-center gap-2">
                      {actor && <Initials id={actor.id} name={actor.name} size={18} />}
                      <span className="truncate text-[12.5px]">{e.title}</span>
                      {e.detail && (
                        <span className="truncate text-[11.5px] text-muted-foreground">
                          · {e.detail}
                        </span>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      {e.taskId && (
                        <Link
                          to="/tasks/$taskId"
                          params={{ taskId: e.taskId }}
                          className="mono-xs text-primary hover:underline"
                        >
                          {e.taskId}
                        </Link>
                      )}
                      {e.runId && (
                        <Link
                          to="/runs/$runId"
                          params={{ runId: e.runId }}
                          className="mono-xs text-primary hover:underline"
                        >
                          {e.runId}
                        </Link>
                      )}
                      <span className="mono-xs w-16 text-right text-muted-foreground">
                        {relTime(e.at)}
                      </span>
                    </div>
                  </li>
                );
              })}
            </ol>
          )}
        </Panel>
      </PageBody>
    </Page>
  );
}
