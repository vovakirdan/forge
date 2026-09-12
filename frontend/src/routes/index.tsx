import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import {
  AgentService,
  ActivityService,
  GoalService,
  ProjectService,
  ResourceService,
  RunService,
  TaskService,
} from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import {
  Chip,
  Dot,
  Initials,
  KV,
  Metric,
  Meter,
  Panel,
  StageBadge,
  priorityTone,
} from "@/components/common/Bits";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { duration, relTime, stageLabel } from "@/lib/format";
import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  CircleDollarSign,
  Clock,
  MessageSquare,
  XCircle,
} from "lucide-react";

export const Route = createFileRoute("/")({
  head: () => ({
    meta: [
      { title: "Control Room — Forge" },
      {
        name: "description",
        content:
          "Live status of the autonomous engineering team: active pipeline stages, risks, failures and pending human decisions.",
      },
      { property: "og:title", content: "Control Room — Forge" },
      {
        property: "og:description",
        content: "What the AI engineering team is doing right now, at a glance.",
      },
    ],
  }),
  component: Overview,
});

function Overview() {
  const { data: project } = useQuery({ queryKey: ["project"], queryFn: ProjectService.current });
  const { data: goals } = useQuery({ queryKey: ["goals"], queryFn: GoalService.list });
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: host } = useQuery({ queryKey: ["host"], queryFn: ResourceService.host });
  const { data: risks } = useQuery({ queryKey: ["risks"], queryFn: ProjectService.risks });
  const { data: events } = useQuery({
    queryKey: ["activity"],
    queryFn: () => ActivityService.list(),
  });
  const { data: runs } = useQuery({ queryKey: ["runs", "active"], queryFn: RunService.active });

  const goal = goals?.[0];
  const activeStages = ["implementation", "verification", "review", "integration"];
  const active = tasks?.filter((t) => activeStages.includes(t.stage)) ?? [];
  const waiting = tasks?.filter((t) => t.stage === "waiting") ?? [];
  const ready = tasks?.filter((t) => t.stage === "ready") ?? [];
  const done = tasks?.filter((t) => t.stage === "done") ?? [];
  const planned = tasks?.filter((t) => t.stage === "planned") ?? [];
  const failures =
    events?.filter((e) => e.category === "verification" && e.title.includes("failed")) ?? [];

  if (!tasks || !agents || !host) return <LoadingSkeleton />;

  return (
    <Page>
      <PageHeader
        title="Control Room"
        subtitle={`${project?.name} · ${project?.repository}`}
        actions={
          <Button asChild size="sm" variant="outline" className="h-7 text-[12px]">
            <Link to="/chat">
              <MessageSquare className="size-3.5" /> Ask the Lead
            </Link>
          </Button>
        }
      />
      <PageBody className="space-y-3">
        {/* metric strip */}
        <div className="panel grid grid-cols-2 divide-x divide-border md:grid-cols-4 xl:grid-cols-7">
          <Metric label="Current goal" value={`${goal?.progress ?? 0}%`} hint={goal?.title} />
          <Metric
            label="Team"
            value={`${agents.length} employees`}
            hint={`${agents.filter((a) => a.state === "working").length} working`}
          />
          <Metric
            label="Active work"
            value={`${active.length} tasks`}
            hint={`${runs?.length ?? 0} runs executing`}
            tone="running"
          />
          <Metric
            label="Waiting"
            value={`${waiting.length} tasks`}
            hint="blocked on decisions"
            tone={waiting.length ? "danger" : "muted"}
          />
          <Metric
            label="Heavy verification"
            value={host.heavyInUse >= host.heavyCapacity ? "Busy" : "Free"}
            hint={`${host.heavyInUse} / ${host.heavyCapacity} · ${host.queues[1]?.waiting ?? 0} queued`}
            tone="warning"
          />
          <Metric
            label="Recent integration"
            value="Passed"
            hint="TASK-139 merged 4c19b02"
            tone="success"
          />
          <Metric
            label="Cost today"
            value={
              <span className="flex items-center gap-1.5 text-[13px]">
                <CircleDollarSign className="size-3.5 text-muted-foreground" />
                Codex 18% · Claude 7%
              </span>
            }
            hint={`$${agents.reduce((s, a) => s + a.costTodayUsd, 0).toFixed(2)} spend`}
          />
        </div>

        {/* goal + pipeline */}
        <div className="grid gap-3 xl:grid-cols-[1fr_320px]">
          <Panel
            title="Pipeline — live"
            action={
              <Link to="/board" className="text-[11px] text-primary hover:underline">
                Open board
              </Link>
            }
            dense
          >
            <div className="divide-y divide-border">
              {[...active, ...ready.slice(0, 1)].map((t) => {
                const assignee = agents.find((a) => a.id === t.assigneeId);
                return (
                  <Link
                    key={t.id}
                    to="/tasks/$taskId"
                    params={{ taskId: t.id }}
                    className="grid grid-cols-[92px_1fr_130px_120px_84px] items-center gap-2 px-3 py-2 transition-colors hover:bg-accent/50"
                  >
                    <span className="mono-xs text-muted-foreground">{t.id}</span>
                    <span className="truncate text-[12.5px] font-medium">{t.title}</span>
                    <StageBadge stage={t.stage} />
                    <span className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
                      {assignee ? (
                        <>
                          <Initials id={assignee.id} name={assignee.name} size={18} />
                          {assignee.name}
                        </>
                      ) : t.stage === "verification" ? (
                        "System"
                      ) : (
                        "Unassigned"
                      )}
                    </span>
                    <span className="mono-xs flex items-center justify-end gap-1 text-muted-foreground">
                      {t.elapsedMin > 0 ? (
                        <>
                          <Clock className="size-3" /> {duration(t.elapsedMin)}
                        </>
                      ) : (
                        "—"
                      )}
                    </span>
                  </Link>
                );
              })}
            </div>
          </Panel>

          <Panel title="Current goal">
            <p className="text-[12.5px] font-medium leading-snug">{goal?.title}</p>
            <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
              {goal?.description}
            </p>
            <div className="mt-3 flex items-center gap-2">
              <Meter value={goal?.progress ?? 0} />
              <span className="mono-xs tabular-nums">{goal?.progress}%</span>
            </div>
            <div className="mt-3 space-y-1.5">{goals && <EpicRows goalId={goal!.id} />}</div>
            <Button asChild size="sm" variant="outline" className="mt-3 h-7 w-full text-[12px]">
              <Link to="/goals">
                Open goal <ArrowRight className="size-3" />
              </Link>
            </Button>
          </Panel>
        </div>

        {/* lower grid */}
        <div className="grid gap-3 lg:grid-cols-3">
          <Panel title="Current risks" dense>
            <ul className="divide-y divide-border">
              {risks?.map((r) => (
                <li key={r.id} className="flex gap-2 px-3 py-2">
                  <AlertTriangle
                    className={
                      r.severity === "high"
                        ? "mt-0.5 size-3.5 text-destructive"
                        : "mt-0.5 size-3.5 text-warning"
                    }
                  />
                  <div className="min-w-0">
                    <p className="text-[12px] font-medium">{r.title}</p>
                    <p className="text-[11px] text-muted-foreground">{r.detail}</p>
                  </div>
                </li>
              ))}
            </ul>
          </Panel>

          <Panel title="Pending human decisions" dense>
            <ul className="divide-y divide-border">
              {waiting.map((t) => (
                <li key={t.id} className="px-3 py-2">
                  <div className="flex items-center gap-2">
                    <Chip tone={priorityTone[t.priority]}>{t.priority}</Chip>
                    <Link
                      to="/tasks/$taskId"
                      params={{ taskId: t.id }}
                      className="mono-xs text-primary hover:underline"
                    >
                      {t.id}
                    </Link>
                  </div>
                  <p className="mt-1 text-[12px] font-medium">{t.title}</p>
                  <p className="text-[11px] text-muted-foreground">{t.blockedReason}</p>
                </li>
              ))}
              <li className="px-3 py-2">
                <Button asChild size="sm" variant="secondary" className="h-6 w-full text-[11.5px]">
                  <Link to="/chat">Answer in chat</Link>
                </Button>
              </li>
            </ul>
          </Panel>

          <Panel title="Recent failures" dense>
            <ul className="divide-y divide-border">
              {failures.map((f) => (
                <li key={f.id} className="flex gap-2 px-3 py-2">
                  <XCircle className="mt-0.5 size-3.5 text-destructive" />
                  <div className="min-w-0 flex-1">
                    <p className="text-[12px] font-medium">{f.title}</p>
                    <p className="text-[11px] text-muted-foreground">
                      {f.detail} · returned to implementation, no new task created
                    </p>
                  </div>
                  <span className="mono-xs shrink-0 text-muted-foreground">{relTime(f.at)}</span>
                </li>
              ))}
              <li className="flex gap-2 px-3 py-2">
                <AlertTriangle className="mt-0.5 size-3.5 text-warning" />
                <div className="min-w-0 flex-1">
                  <p className="text-[12px] font-medium">Review changes requested TASK-144</p>
                  <p className="text-[11px] text-muted-foreground">
                    Alice · same task stays in pipeline
                  </p>
                </div>
              </li>
            </ul>
          </Panel>
        </div>

        <div className="grid gap-3 lg:grid-cols-2">
          <Panel title="Recently completed" dense>
            <ul className="divide-y divide-border">
              {done.map((t) => (
                <li key={t.id} className="flex items-center gap-2 px-3 py-2">
                  <CheckCircle2 className="size-3.5 text-success" />
                  <Link
                    to="/tasks/$taskId"
                    params={{ taskId: t.id }}
                    className="mono-xs text-muted-foreground hover:text-primary"
                  >
                    {t.id}
                  </Link>
                  <span className="min-w-0 flex-1 truncate text-[12px]">{t.title}</span>
                  <span className="mono-xs text-muted-foreground">{relTime(t.updatedAt)}</span>
                </li>
              ))}
            </ul>
          </Panel>

          <Panel title="Upcoming (planned wave)" dense>
            <ul className="divide-y divide-border">
              {[...ready, ...planned].map((t) => (
                <li key={t.id} className="flex items-center gap-2 px-3 py-2">
                  <Dot tone={t.stage === "ready" ? "info" : "muted"} />
                  <Link
                    to="/tasks/$taskId"
                    params={{ taskId: t.id }}
                    className="mono-xs text-muted-foreground hover:text-primary"
                  >
                    {t.id}
                  </Link>
                  <span className="min-w-0 flex-1 truncate text-[12px]">{t.title}</span>
                  <Chip tone={priorityTone[t.priority]}>{t.priority}</Chip>
                  <StageBadge stage={t.stage} />
                </li>
              ))}
            </ul>
          </Panel>
        </div>
      </PageBody>
    </Page>
  );
}

function EpicRows({ goalId }: { goalId: string }) {
  const { data: epics } = useQuery({
    queryKey: ["epics", goalId],
    queryFn: () => GoalService.epics(goalId),
  });
  return (
    <>
      {epics?.map((e) => (
        <div key={e.id}>
          <KV k={e.title} v={`${e.progress}%`} />
          <Meter value={e.progress} tone={e.progress === 0 ? "muted" : "primary"} className="h-1" />
        </div>
      ))}
    </>
  );
}

function LoadingSkeleton() {
  return (
    <PageBody className="space-y-3">
      <Skeleton className="h-16 w-full" />
      <Skeleton className="h-52 w-full" />
      <div className="grid gap-3 lg:grid-cols-3">
        <Skeleton className="h-40" />
        <Skeleton className="h-40" />
        <Skeleton className="h-40" />
      </div>
    </PageBody>
  );
}
