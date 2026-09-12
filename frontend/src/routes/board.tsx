import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { AgentService, GoalService, PipelineService, TaskService } from "@/services";
import type { StageId, Task } from "@/data/types";
import { Page, PageHeader } from "@/components/common/Page";
import { Chip, Dot, EmptyState, Hint, Initials, priorityTone } from "@/components/common/Bits";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { duration, stageLabel } from "@/lib/format";
import {
  CheckCircle2,
  Clock,
  Columns3,
  Filter,
  GitBranch,
  MessageSquare,
  Play,
  RotateCcw,
  ScanEye,
  ShieldAlert,
  XCircle,
} from "lucide-react";
import { TaskDrawer } from "@/components/task/TaskDrawer";

export const Route = createFileRoute("/board")({
  head: () => ({
    meta: [
      { title: "Engineering Board — Forge" },
      {
        name: "description",
        content:
          "Kanban board of executable work items moving through implementation, verification, review and integration.",
      },
      { property: "og:title", content: "Engineering Board — Forge" },
      {
        property: "og:description",
        content: "One task stays one task across its whole pipeline.",
      },
    ],
  }),
  component: Board,
});

const COLUMNS: StageId[] = [
  "planned",
  "ready",
  "implementation",
  "verification",
  "review",
  "integration",
  "waiting",
  "done",
];

const ALL = "all";

function Board() {
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: epics } = useQuery({ queryKey: ["epics"], queryFn: () => GoalService.epics() });
  const { data: pipelines } = useQuery({ queryKey: ["pipelines"], queryFn: PipelineService.list });

  const [assignee, setAssignee] = useState(ALL);
  const [epic, setEpic] = useState(ALL);
  const [priority, setPriority] = useState(ALL);
  const [pipeline, setPipeline] = useState(ALL);
  const [area, setArea] = useState(ALL);
  const [blockedOnly, setBlockedOnly] = useState(false);
  const [openTask, setOpenTask] = useState<string | null>(null);

  const areas = useMemo(() => [...new Set(tasks?.map((t) => t.area) ?? [])], [tasks]);

  const filtered = useMemo(
    () =>
      (tasks ?? []).filter(
        (t) =>
          (assignee === ALL || t.assigneeId === assignee) &&
          (epic === ALL || t.epicId === epic) &&
          (priority === ALL || t.priority === priority) &&
          (pipeline === ALL || t.pipelineId === pipeline) &&
          (area === ALL || t.area === area) &&
          (!blockedOnly || t.blocked || t.stage === "waiting"),
      ),
    [tasks, assignee, epic, priority, pipeline, area, blockedOnly],
  );

  const reset = () => {
    setAssignee(ALL);
    setEpic(ALL);
    setPriority(ALL);
    setPipeline(ALL);
    setArea(ALL);
    setBlockedOnly(false);
  };

  return (
    <Page>
      <PageHeader
        title="Engineering Board"
        subtitle={`${filtered.length} executable work items · ${filtered.filter((t) => ["implementation", "verification", "review"].includes(t.stage)).length} in flight · backlog size is independent of runtime concurrency`}
        actions={
          <>
            <Chip tone="muted" icon={<Filter className="size-2.5" />}>
              Work items only
            </Chip>
            <Button variant="ghost" size="sm" className="h-7 text-[12px]" onClick={reset}>
              <RotateCcw className="size-3.5" /> Reset
            </Button>
          </>
        }
        tabs={
          <div className="flex flex-wrap items-center gap-1.5">
            <FilterSelect
              value={assignee}
              onChange={setAssignee}
              placeholder="Employee"
              options={(agents ?? []).map((a) => ({ value: a.id, label: a.name }))}
            />
            <FilterSelect
              value={epic}
              onChange={setEpic}
              placeholder="Epic"
              options={(epics ?? []).map((e) => ({ value: e.id, label: e.title }))}
            />
            <FilterSelect
              value={priority}
              onChange={setPriority}
              placeholder="Priority"
              options={["critical", "high", "medium", "low"].map((p) => ({ value: p, label: p }))}
            />
            <FilterSelect
              value={pipeline}
              onChange={setPipeline}
              placeholder="Pipeline"
              options={(pipelines ?? []).map((p) => ({ value: p.id, label: p.name }))}
            />
            <FilterSelect
              value={area}
              onChange={setArea}
              placeholder="Area"
              options={areas.map((a) => ({ value: a, label: a }))}
            />
            <Button
              size="sm"
              variant={blockedOnly ? "default" : "outline"}
              className="h-7 text-[11.5px]"
              onClick={() => setBlockedOnly((v) => !v)}
            >
              <ShieldAlert className="size-3.5" /> Blocked / waiting
            </Button>
          </div>
        }
      />

      {!tasks ? (
        <div className="flex gap-3 p-4">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={i} className="h-72 w-64 shrink-0" />
          ))}
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-x-auto">
          <div className="flex h-full min-w-max gap-2.5 p-3">
            {COLUMNS.map((stage) => {
              const items = filtered.filter((t) => t.stage === stage);
              return (
                <div key={stage} className="flex h-full w-[268px] shrink-0 flex-col">
                  <div className="mb-1.5 flex items-center gap-1.5 px-1">
                    <Dot
                      tone={stage === "waiting" ? "danger" : stage === "done" ? "success" : "muted"}
                    />
                    <span className="section-label">{stageLabel[stage]}</span>
                    <span className="mono-xs text-muted-foreground">{items.length}</span>
                  </div>
                  <div className="min-h-0 flex-1 space-y-2 overflow-auto rounded-lg border border-border/70 bg-surface/60 p-1.5">
                    {items.length === 0 ? (
                      <EmptyState
                        icon={<Columns3 className="size-4" />}
                        title="No work items"
                        hint={
                          stage === "integration"
                            ? "Only integration may merge to main."
                            : undefined
                        }
                      />
                    ) : (
                      items.map((t) => (
                        <TaskCard
                          key={t.id}
                          task={t}
                          agents={agents ?? []}
                          epicTitle={epics?.find((e) => e.id === t.epicId)?.title ?? ""}
                          pipelineName={pipelines?.find((p) => p.id === t.pipelineId)?.name ?? ""}
                          onOpen={() => setOpenTask(t.id)}
                        />
                      ))
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      <TaskDrawer taskId={openTask} onClose={() => setOpenTask(null)} />
    </Page>
  );
}

function FilterSelect({
  value,
  onChange,
  placeholder,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  options: { value: string; label: string }[];
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="h-7 w-auto min-w-[110px] gap-1 text-[11.5px]">
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL} className="text-[12px]">
          {placeholder}: all
        </SelectItem>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value} className="text-[12px] capitalize">
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function TaskCard({
  task,
  agents,
  epicTitle,
  pipelineName,
  onOpen,
}: {
  task: Task;
  agents: { id: string; name: string }[];
  epicTitle: string;
  pipelineName: string;
  onOpen: () => void;
}) {
  const assignee = agents.find((a) => a.id === task.assigneeId);
  return (
    <button
      onClick={onOpen}
      className="w-full rounded-md border border-border bg-card p-2 text-left transition-colors hover:border-border-strong hover:bg-accent/40"
    >
      <div className="flex items-center gap-1.5">
        <span className="mono-xs text-muted-foreground">{task.id}</span>
        <Chip tone={priorityTone[task.priority]}>{task.priority}</Chip>
        {task.blocked && (
          <Hint label={task.blockedReason ?? "Blocked"}>
            <span>
              <ShieldAlert className="size-3 text-destructive" />
            </span>
          </Hint>
        )}
        {task.attempts > 0 && (
          <span className="mono-xs ml-auto text-muted-foreground">att {task.attempts}</span>
        )}
      </div>

      <p className="mt-1 line-clamp-2 text-[12.5px] font-medium leading-snug">{task.title}</p>

      <div className="mt-1.5 flex flex-wrap items-center gap-1">
        <Chip tone="muted">{epicTitle}</Chip>
        <Chip tone="muted" icon={<GitBranch className="size-2.5" />}>
          {pipelineName}
        </Chip>
      </div>

      <div className="mono-xs mt-1.5 flex items-center gap-2 text-muted-foreground">
        <span>base {task.baseSha}</span>
        <span className="truncate">{task.workspace}</span>
      </div>

      <div className="mt-2 flex items-center gap-2 border-t border-border pt-1.5">
        {assignee ? (
          <span className="flex items-center gap-1 text-[11px]">
            <Initials id={assignee.id} name={assignee.name} size={16} />
            {assignee.name}
          </span>
        ) : (
          <span className="text-[11px] text-muted-foreground">Unassigned</span>
        )}
        <div className="ml-auto flex items-center gap-1.5 text-muted-foreground">
          <Hint label={`Verification: ${task.verification}`}>
            <span>
              {task.verification === "passed" ? (
                <CheckCircle2 className="size-3 text-success" />
              ) : task.verification === "failed" ? (
                <XCircle className="size-3 text-destructive" />
              ) : task.verification === "running" ? (
                <Play className="size-3 text-running" />
              ) : (
                <CheckCircle2 className="size-3 opacity-30" />
              )}
            </span>
          </Hint>
          <Hint label={`Review: ${task.review}`}>
            <span>
              <ScanEye
                className={
                  task.review === "approved"
                    ? "size-3 text-success"
                    : task.review === "changes_requested"
                      ? "size-3 text-warning"
                      : task.review === "in_review"
                        ? "size-3 text-running"
                        : "size-3 opacity-30"
                }
              />
            </span>
          </Hint>
          {task.findingIds.length > 0 && (
            <Hint label={`${task.findingIds.length} findings`}>
              <span className="mono-xs flex items-center gap-0.5">
                <ShieldAlert className="size-3" />
                {task.findingIds.length}
              </span>
            </Hint>
          )}
          {task.commentCount > 0 && (
            <span className="mono-xs flex items-center gap-0.5">
              <MessageSquare className="size-3" />
              {task.commentCount}
            </span>
          )}
          {task.elapsedMin > 0 && (
            <span className="mono-xs flex items-center gap-0.5">
              <Clock className="size-3" />
              {duration(task.elapsedMin)}
            </span>
          )}
        </div>
      </div>

      {task.currentRunId && (
        <div className="mono-xs mt-1.5 flex items-center gap-1 rounded bg-running/10 px-1.5 py-0.5 text-running">
          <Dot tone="running" pulse />
          {task.currentRunId} executing
        </div>
      )}
    </button>
  );
}

export { Link };
