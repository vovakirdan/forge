import { createFileRoute, Link, useParams } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { RunService, AgentService, TaskService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, EmptyState, Initials, KV, Meter, Panel } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { clockTime, durationSec, engineLabel } from "@/lib/format";
import { FileDiff, Square, Terminal } from "lucide-react";

export const Route = createFileRoute("/runs/$runId")({
  head: ({ params }) => ({
    meta: [
      { title: `${params.runId} — Run detail — Forge` },
      {
        name: "description",
        content:
          "Observable actions, commands, files changed, output and resource usage for a single agent run.",
      },
      { property: "og:title", content: "Run detail — Forge" },
      { property: "og:description", content: "Observable actions only — no hidden reasoning." },
    ],
  }),
  component: RunDetail,
});

const cmdTone = { ok: "success", failed: "danger", running: "running" } as const;

function RunDetail() {
  const { runId } = useParams({ from: "/runs/$runId" });
  const { data: run, isLoading } = useQuery({
    queryKey: ["run", runId],
    queryFn: () => RunService.get(runId),
  });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });

  if (isLoading) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-72 w-full" />
      </PageBody>
    );
  }
  if (!run) {
    return (
      <PageBody>
        <Panel>
          <EmptyState
            title="Run not found"
            hint="Runs are retained for 30 days after completion."
          />
        </Panel>
      </PageBody>
    );
  }

  const agent = agents?.find((a) => a.id === run.agentId);
  const task = tasks?.find((t) => t.id === run.taskId);
  const tone =
    run.status === "running"
      ? "running"
      : run.status === "failed"
        ? "danger"
        : run.status === "queued"
          ? "muted"
          : "success";

  return (
    <Page>
      <PageHeader
        title={
          <span className="flex items-center gap-2">
            <span className="font-mono">{run.id}</span>
            <Chip tone={tone}>
              <Dot tone={tone} pulse={run.status === "running"} />
              {run.status}
            </Chip>
          </span>
        }
        subtitle={run.reason}
        actions={
          run.status === "running" ? (
            <Button
              size="sm"
              variant="destructive"
              className="h-7 text-[12px]"
              onClick={() =>
                toast.warning("Run aborted", {
                  description: `${run.taskId} returns to its previous stage.`,
                })
              }
            >
              <Square className="size-3.5" /> Abort run
            </Button>
          ) : null
        }
      />
      <PageBody className="grid gap-3 xl:grid-cols-[300px_1fr]">
        <div className="space-y-3">
          <Panel title="Run">
            <div className="grid gap-x-4">
              <KV
                k="Agent"
                v={
                  agent ? (
                    <Link
                      to="/team/$agentId"
                      params={{ agentId: agent.id }}
                      className="inline-flex items-center gap-1.5 text-primary hover:underline"
                    >
                      <Initials id={agent.id} name={agent.name} size={16} /> {agent.name}
                    </Link>
                  ) : (
                    "System"
                  )
                }
              />
              <KV k="Engine" v={engineLabel[run.engine]} />
              <KV
                k="Task"
                v={
                  <Link
                    to="/tasks/$taskId"
                    params={{ taskId: run.taskId }}
                    className="text-primary hover:underline"
                  >
                    {run.taskId}
                  </Link>
                }
              />
              <KV k="Title" v={task?.title ?? "—"} />
              <KV k="Started" v={clockTime(run.startedAt)} />
              <KV k="Duration" v={durationSec(run.durationSec)} />
              <KV k="Resource class" v={run.resourceClass} />
              <KV k="Workspace" v={run.workspace} />
            </div>
          </Panel>

          <Panel title="Usage">
            <div className="grid gap-x-4">
              <KV k="Tokens in" v={run.tokensIn.toLocaleString()} />
              <KV k="Tokens out" v={run.tokensOut.toLocaleString()} />
            </div>
            <div className="mt-2 space-y-2">
              <div>
                <div className="flex justify-between text-[11.5px]">
                  <span>CPU</span>
                  <span className="mono-xs">{run.cpuPct}%</span>
                </div>
                <Meter
                  value={run.cpuPct}
                  tone={run.cpuPct > 75 ? "warning" : "primary"}
                  className="mt-1 h-1"
                />
              </div>
              <div>
                <div className="flex justify-between text-[11.5px]">
                  <span>RAM</span>
                  <span className="mono-xs">{run.ramGb} GB</span>
                </div>
                <Meter value={(run.ramGb / 16) * 100} tone="info" className="mt-1 h-1" />
              </div>
            </div>
          </Panel>

          <Panel title="Context provided">
            <ul className="space-y-1 text-[12px]">
              {run.context.map((c) => (
                <li key={c} className="flex gap-1.5">
                  <Dot tone="info" />
                  <span className="leading-tight">{c}</span>
                </li>
              ))}
            </ul>
          </Panel>

          <Panel title="Tools used">
            <div className="flex flex-wrap gap-1">
              {run.tools.map((t) => (
                <Chip key={t} tone="muted" mono>
                  {t}
                </Chip>
              ))}
            </div>
          </Panel>
        </div>

        <div className="space-y-3">
          <Panel title="Command timeline" dense>
            <ol className="divide-y divide-border">
              {run.commands.map((c) => (
                <li
                  key={c.id}
                  className="grid grid-cols-[54px_84px_1fr_60px] items-center gap-3 px-3 py-1.5"
                >
                  <span className="mono-xs tabular-nums text-muted-foreground">
                    {clockTime(c.at)}
                  </span>
                  <Chip tone={cmdTone[c.status]}>
                    <Terminal className="size-2.5" />
                    {c.tool}
                  </Chip>
                  <code className="truncate font-mono text-[11.5px]">{c.command}</code>
                  <span className="mono-xs text-right text-muted-foreground">
                    {durationSec(c.durationSec)}
                  </span>
                </li>
              ))}
            </ol>
          </Panel>

          <Panel title="Files changed" dense>
            <ul className="divide-y divide-border">
              {run.filesChanged.map((f) => (
                <li key={f.path} className="flex items-center gap-3 px-3 py-1.5">
                  <FileDiff className="size-3 text-muted-foreground" />
                  <code className="min-w-0 flex-1 truncate font-mono text-[11.5px]">{f.path}</code>
                  <span className="mono-xs text-success">+{f.added}</span>
                  <span className="mono-xs text-destructive">-{f.removed}</span>
                </li>
              ))}
            </ul>
          </Panel>

          <Panel title="Agent output">
            <div className="space-y-1.5">
              {run.output.map((line, i) => (
                <p
                  key={i}
                  className="border-l-2 border-border pl-2 text-[12px] leading-relaxed text-muted-foreground"
                >
                  {line}
                </p>
              ))}
            </div>
            <p className="mt-3 text-[11px] text-muted-foreground">
              Observable actions and summaries only. Internal model reasoning is never surfaced.
            </p>
          </Panel>
        </div>
      </PageBody>
    </Page>
  );
}
