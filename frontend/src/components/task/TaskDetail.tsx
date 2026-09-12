import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { toast } from "sonner";
import { AgentService, GoalService, PipelineService, RunService, TaskService } from "@/services";
import type { Task, TimelineEntry } from "@/data/types";
import {
  Chip,
  Dot,
  Initials,
  KV,
  Meter,
  Panel,
  StageBadge,
  priorityTone,
} from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { duration, relTime, stageLabel } from "@/lib/format";
import {
  ArrowDown,
  CheckCircle2,
  ChevronDown,
  CircleDot,
  FileCode2,
  FileText,
  FlaskConical,
  GitCommit,
  Hammer,
  Loader2,
  MessageSquare,
  ScanEye,
  ScrollText,
  ShieldAlert,
  XCircle,
} from "lucide-react";

export function TaskSummaryBar({ task }: { task: Task }) {
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: pipelines } = useQuery({ queryKey: ["pipelines"], queryFn: PipelineService.list });
  const { data: goals } = useQuery({ queryKey: ["goals"], queryFn: GoalService.list });
  const { data: epics } = useQuery({ queryKey: ["epics"], queryFn: () => GoalService.epics() });

  const assignee = agents?.find((a) => a.id === task.assigneeId);
  const reviewer = agents?.find((a) => a.id === task.reviewerId);

  return (
    <div className="grid grid-cols-2 gap-x-5 md:grid-cols-4">
      <KV k="Goal" v={goals?.find((g) => g.id === task.goalId)?.title ?? "—"} />
      <KV k="Epic" v={epics?.find((e) => e.id === task.epicId)?.title ?? "—"} />
      <KV
        k="Assignee"
        v={
          assignee ? (
            <Link
              to="/team/$agentId"
              params={{ agentId: assignee.id }}
              className="inline-flex items-center gap-1 hover:text-primary"
            >
              <Initials id={assignee.id} name={assignee.name} size={15} /> {assignee.name}
            </Link>
          ) : (
            "Unassigned"
          )
        }
      />
      <KV k="Reviewer" v={reviewer?.name ?? "—"} />
      <KV k="Priority" v={<Chip tone={priorityTone[task.priority]}>{task.priority}</Chip>} />
      <KV k="Pipeline" v={pipelines?.find((p) => p.id === task.pipelineId)?.name ?? "—"} />
      <KV k="Base SHA" v={<span className="font-mono">{task.baseSha}</span>} />
      <KV k="Workspace" v={<span className="font-mono">{task.workspace}</span>} />
      <KV k="Created" v={relTime(task.createdAt)} />
      <KV k="Current stage" v={<StageBadge stage={task.stage} />} />
      <KV k="Attempts" v={`${task.attempts}`} />
      <KV k="Comments" v={`${task.commentCount}`} />
    </div>
  );
}

export function TaskDetailBody({ task }: { task: Task }) {
  const { data: comments } = useQuery({
    queryKey: ["comments", task.id],
    queryFn: () => TaskService.comments(task.id),
  });
  const { data: artifacts } = useQuery({
    queryKey: ["artifacts", task.id],
    queryFn: () => TaskService.artifacts(task.id),
  });
  const { data: findings } = useQuery({
    queryKey: ["findings", task.id],
    queryFn: () => TaskService.findings(task.id),
  });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: runs } = useQuery({
    queryKey: ["runs", task.id],
    queryFn: () => RunService.forTask(task.id),
  });
  const { data: epics } = useQuery({ queryKey: ["epics"], queryFn: () => GoalService.epics() });
  const { data: goals } = useQuery({ queryKey: ["goals"], queryFn: GoalService.list });

  const dodDone = task.dod.filter((d) => d.done).length;

  return (
    <div className="space-y-3">
      <Panel title="Description">
        <p className="text-[12.5px] leading-relaxed text-foreground/90">{task.description}</p>
      </Panel>

      <div className="grid gap-3 lg:grid-cols-2">
        <Panel title="Why — ancestry">
          <ol className="space-y-1">
            {[
              {
                label: goals?.find((g) => g.id === task.goalId)?.title ?? "Global goal",
                kind: "Global goal",
              },
              { label: epics?.find((e) => e.id === task.epicId)?.title ?? "Epic", kind: "Epic" },
              { label: task.parentTheme, kind: "Theme" },
              { label: `${task.id} — ${task.title}`, kind: "Work item" },
            ].map((n, i, arr) => (
              <li key={n.kind}>
                <div className="flex items-start gap-2">
                  <span className="section-label mt-0.5 w-20 shrink-0">{n.kind}</span>
                  <span
                    className={
                      i === arr.length - 1
                        ? "text-[12.5px] font-semibold"
                        : "text-[12.5px] text-foreground/85"
                    }
                  >
                    {n.label}
                  </span>
                </div>
                {i < arr.length - 1 && (
                  <ArrowDown className="ml-[76px] size-3 text-muted-foreground" />
                )}
              </li>
            ))}
          </ol>
        </Panel>

        <Panel
          title={`Definition of done — ${dodDone}/${task.dod.length}`}
          action={<Meter value={(dodDone / task.dod.length) * 100} className="w-24" />}
        >
          <ul className="space-y-1.5">
            {task.dod.map((d) => (
              <li key={d.id} className="flex items-center gap-2">
                <Checkbox checked={d.done} disabled className="size-3.5" />
                <span
                  className={
                    d.done ? "text-[12px] text-muted-foreground line-through" : "text-[12px]"
                  }
                >
                  {d.label}
                </span>
                {d.automated && (
                  <Chip tone="muted" className="ml-auto">
                    machine-checked
                  </Chip>
                )}
              </li>
            ))}
          </ul>
        </Panel>
      </div>

      <Panel
        title="Pipeline timeline"
        action={
          <span className="text-[10.5px] text-muted-foreground">
            Failures return this task to an earlier stage — they never create a new task
          </span>
        }
      >
        {task.timeline.length === 0 ? (
          <p className="text-[12px] text-muted-foreground">
            Not started. The task enters implementation once the scheduler allocates a run slot.
          </p>
        ) : (
          <ol className="relative space-y-2 border-l border-border pl-4">
            {task.timeline.map((e) => (
              <TimelineRow key={e.id} entry={e} />
            ))}
          </ol>
        )}
      </Panel>

      <div className="grid gap-3 lg:grid-cols-2">
        <Panel title={`Findings (${findings?.length ?? 0})`} dense>
          {findings?.length ? (
            <ul className="divide-y divide-border">
              {findings.map((f) => {
                const by = agents?.find((a) => a.id === f.foundById);
                return (
                  <li key={f.id} className="px-3 py-2">
                    <div className="flex items-center gap-2">
                      <span className="mono-xs text-muted-foreground">{f.id}</span>
                      <Chip
                        tone={
                          f.severity === "high"
                            ? "danger"
                            : f.severity === "medium"
                              ? "warning"
                              : "muted"
                        }
                      >
                        {f.severity}
                      </Chip>
                      <Chip tone={f.status === "promoted" ? "success" : "muted"}>{f.status}</Chip>
                      <span className="ml-auto text-[11px] text-muted-foreground">
                        found by {by?.name}
                      </span>
                    </div>
                    <p className="mt-1 text-[12px] font-medium">{f.title}</p>
                    <p className="text-[11.5px] text-muted-foreground">{f.description}</p>
                    {f.status === "open" ? (
                      <div className="mt-1.5 flex gap-1.5">
                        <Button
                          size="sm"
                          className="h-6 text-[11px]"
                          onClick={() =>
                            toast.success(`${f.id} promoted`, {
                              description: "Only the Lead or you can promote findings.",
                            })
                          }
                        >
                          Promote to task
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 text-[11px]"
                          onClick={() => toast(`${f.id} ignored`)}
                        >
                          Ignore
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="h-6 text-[11px]"
                          onClick={() => toast(`${f.id} attached to an existing task`)}
                        >
                          Attach to existing task
                        </Button>
                      </div>
                    ) : (
                      f.promotedTaskId && (
                        <p className="mt-1 text-[11px]">
                          Promoted into{" "}
                          <Link
                            to="/tasks/$taskId"
                            params={{ taskId: f.promotedTaskId }}
                            className="text-primary hover:underline"
                          >
                            {f.promotedTaskId}
                          </Link>
                        </p>
                      )
                    )}
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="px-3 py-4 text-[12px] text-muted-foreground">
              No findings reported on this task.
            </p>
          )}
        </Panel>

        <Panel title={`Artifacts (${artifacts?.length ?? 0})`} dense>
          <ul className="divide-y divide-border">
            {artifacts?.map((a) => (
              <li key={a.id} className="flex items-center gap-2 px-3 py-2">
                <ArtifactIcon kind={a.kind} />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[12px] font-medium">{a.title}</p>
                  <p className="mono-xs truncate text-muted-foreground">{a.meta}</p>
                </div>
                <Chip tone="muted">{a.kind.replace("_", " ")}</Chip>
              </li>
            ))}
          </ul>
        </Panel>
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <Panel title="Runs" dense>
          <ul className="divide-y divide-border">
            {runs?.length ? (
              runs.map((r) => (
                <li key={r.id}>
                  <Link
                    to="/runs/$runId"
                    params={{ runId: r.id }}
                    className="flex items-center gap-2 px-3 py-2 hover:bg-accent/50"
                  >
                    <Dot
                      tone={
                        r.status === "running"
                          ? "running"
                          : r.status === "failed"
                            ? "danger"
                            : "success"
                      }
                      pulse={r.status === "running"}
                    />
                    <span className="mono-xs text-muted-foreground">{r.id}</span>
                    <span className="min-w-0 flex-1 truncate text-[12px]">{r.reason}</span>
                    <Chip tone="muted">{r.resourceClass}</Chip>
                    <span className="mono-xs text-muted-foreground">
                      {Math.round(r.durationSec / 60)}m
                    </span>
                  </Link>
                </li>
              ))
            ) : (
              <li className="px-3 py-4 text-[12px] text-muted-foreground">No runs yet.</li>
            )}
          </ul>
        </Panel>

        <Panel title="Comments" dense>
          <ul className="divide-y divide-border">
            {comments?.length ? (
              comments.map((c) => {
                const author = agents?.find((a) => a.id === c.authorId);
                return (
                  <li key={c.id} className="flex gap-2 px-3 py-2">
                    <Initials id={c.authorId} name={author?.name ?? "?"} size={20} />
                    <div className="min-w-0">
                      <p className="flex items-center gap-1.5 text-[11.5px]">
                        <span className="font-medium">{author?.name}</span>
                        <Chip tone={c.kind === "review" ? "warning" : "muted"}>{c.kind}</Chip>
                        <span className="text-muted-foreground">{relTime(c.createdAt)}</span>
                      </p>
                      <p className="mt-0.5 text-[12px] leading-relaxed">{c.body}</p>
                    </div>
                  </li>
                );
              })
            ) : (
              <li className="px-3 py-4 text-[12px] text-muted-foreground">No comments.</li>
            )}
          </ul>
        </Panel>
      </div>
    </div>
  );
}

function TimelineRow({ entry }: { entry: TimelineEntry }) {
  const tone =
    entry.status === "failed"
      ? "danger"
      : entry.status === "changes_requested"
        ? "warning"
        : entry.status === "in_progress"
          ? "running"
          : "success";
  const Icon =
    entry.stage === "implementation"
      ? Hammer
      : entry.stage === "verification"
        ? FlaskConical
        : entry.stage === "review"
          ? ScanEye
          : GitCommit;

  return (
    <li className="relative">
      <span className="absolute -left-[22px] top-1 grid size-3.5 place-items-center rounded-full border border-border bg-background">
        {entry.status === "in_progress" ? (
          <Loader2 className="size-2.5 animate-spin text-running" />
        ) : entry.status === "failed" ? (
          <XCircle className="size-2.5 text-destructive" />
        ) : entry.status === "changes_requested" ? (
          <CircleDot className="size-2.5 text-warning" />
        ) : (
          <CheckCircle2 className="size-2.5 text-success" />
        )}
      </span>
      <div className="rounded-md border border-border bg-surface px-2.5 py-1.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <Icon className="size-3 text-muted-foreground" />
          <span className="text-[12px] font-semibold capitalize">
            {stageLabel[entry.stage]} #{entry.attempt}
          </span>
          <Chip tone={tone}>{entry.status.replace("_", " ")}</Chip>
          <span className="text-[11px] text-muted-foreground">{entry.actorLabel}</span>
          <span className="mono-xs ml-auto text-muted-foreground">
            {duration(entry.durationMin)} · {relTime(entry.startedAt)}
          </span>
        </div>
        <p className="mt-1 text-[12px]">{entry.summary}</p>
        {entry.details && (
          <ul className="mono-xs mt-1 space-y-0.5 text-muted-foreground">
            {entry.details.map((d) => (
              <li key={d}>· {d}</li>
            ))}
          </ul>
        )}
        {entry.runId && (
          <Link
            to="/runs/$runId"
            params={{ runId: entry.runId }}
            className="mono-xs mt-1 inline-block text-primary hover:underline"
          >
            {entry.runId} →
          </Link>
        )}
      </div>
    </li>
  );
}

function ArtifactIcon({ kind }: { kind: string }) {
  const map: Record<string, typeof FileText> = {
    commit: GitCommit,
    patch: FileCode2,
    test_report: FlaskConical,
    review_report: ScanEye,
    research: ScrollText,
    adr: FileText,
    log: ScrollText,
  };
  const Icon = map[kind] ?? FileText;
  return <Icon className="size-3.5 shrink-0 text-muted-foreground" />;
}

export function TaskActions({ task }: { task: Task }) {
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        className="h-7 text-[12px]"
        onClick={() => toast("Message sent to assignee")}
      >
        <MessageSquare className="size-3.5" /> Message assignee
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm" className="h-7 text-[12px]">
            Stage actions <ChevronDown className="size-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56 text-[12px]">
          <DropdownMenuItem onSelect={() => toast(`${task.id} re-queued for verification`)}>
            Re-run verification
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => toast(`${task.id} sent back to implementation`)}>
            Return to implementation
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => toast(`${task.id} review reassigned`)}>
            Reassign reviewer
          </DropdownMenuItem>
          <Separator className="my-1" />
          <DropdownMenuItem
            className="text-destructive"
            onSelect={() =>
              toast.error(`${task.id} paused`, { description: "Workspace preserved." })
            }
          >
            <ShieldAlert className="size-3.5" /> Pause task
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}
