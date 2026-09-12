import { createFileRoute, Link, useParams } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { AgentService, TaskService, KnowledgeService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import {
  Chip,
  Dot,
  EmptyState,
  Initials,
  KV,
  Meter,
  Panel,
  agentStateTone,
} from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { toast } from "sonner";
import { engineLabel } from "@/lib/format";
import {
  MessageSquare,
  ClipboardPlus,
  PauseCircle,
  Pencil,
  Cpu,
  Wrench,
  UserX,
} from "lucide-react";

export const Route = createFileRoute("/team/$agentId")({
  head: ({ params }) => ({
    meta: [
      { title: `Employee ${params.agentId} — Forge` },
      {
        name: "description",
        content:
          "AI employee profile: role, engine, responsibilities, restrictions and project familiarity.",
      },
      { property: "og:title", content: "Employee profile — Forge" },
      { property: "og:description", content: "What this AI employee may and may not do." },
    ],
  }),
  component: AgentProfile,
});

function AgentProfile() {
  const { agentId } = useParams({ from: "/team/$agentId" });
  const { data: agent, isLoading } = useQuery({
    queryKey: ["agent", agentId],
    queryFn: () => AgentService.get(agentId),
  });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });
  const { data: packs } = useQuery({
    queryKey: ["skillPacks"],
    queryFn: KnowledgeService.skillPacks,
  });
  const { data: lessons } = useQuery({ queryKey: ["lessons"], queryFn: KnowledgeService.lessons });

  if (isLoading) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-72 w-full" />
      </PageBody>
    );
  }
  if (!agent) {
    return (
      <PageBody>
        <Panel>
          <EmptyState
            title="Employee not found"
            hint="They may have been dismissed from the team."
          />
        </Panel>
      </PageBody>
    );
  }

  const manager = agents?.find((a) => a.id === agent.managerId);
  const reports = agents?.filter((a) => a.managerId === agent.id) ?? [];
  const current = tasks?.find((t) => t.id === agent.currentTaskId);
  const agentLessons = lessons?.filter((l) => agent.lessons.includes(l.id)) ?? [];

  return (
    <Page>
      <PageHeader
        title={
          <span className="flex items-center gap-2">
            <Initials id={agent.id} name={agent.name} size={24} />
            {agent.name}
            <Chip tone={agentStateTone[agent.state]}>
              <Dot tone={agentStateTone[agent.state]} pulse={agent.state === "working"} />
              {agent.state}
            </Chip>
          </span>
        }
        subtitle={`${agent.role} · ${engineLabel[agent.engine]} · reports to ${manager?.name ?? "human supervisor"}`}
        actions={
          <div className="flex gap-1.5">
            <Button size="sm" variant="outline" className="h-7 text-[12px]" asChild>
              <Link to="/chat" search={{ channel: `ch-${agent.id}` }}>
                <MessageSquare className="size-3.5" /> Message
              </Link>
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-[12px]"
              onClick={() =>
                toast("Assign work", { description: "Pick a ready work item from the board." })
              }
            >
              <ClipboardPlus className="size-3.5" /> Assign work
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-[12px]"
              onClick={() => toast("Role editor opened")}
            >
              <Pencil className="size-3.5" /> Edit role
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-[12px]"
              onClick={() => toast("Skill configuration opened")}
            >
              <Wrench className="size-3.5" /> Skills
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-[12px]"
              onClick={() =>
                toast("Engine replacement", {
                  description: "Current runs finish on the old engine.",
                })
              }
            >
              <Cpu className="size-3.5" /> Replace engine
            </Button>
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button size="sm" variant="destructive" className="h-7 text-[12px]">
                  <PauseCircle className="size-3.5" /> Suspend
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle className="text-[14px]">Suspend {agent.name}?</AlertDialogTitle>
                  <AlertDialogDescription className="text-[12.5px]">
                    The current run is aborted, the task returns to its previous stage and the
                    workspace is preserved. No work items are deleted.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel className="h-7 text-[12px]">Cancel</AlertDialogCancel>
                  <AlertDialogAction
                    className="h-7 text-[12px]"
                    onClick={() => toast.warning(`${agent.name} suspended`)}
                  >
                    Suspend
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        }
      />
      <PageBody className="grid gap-3 xl:grid-cols-[320px_1fr]">
        <div className="space-y-3">
          <Panel title="Profile">
            <div className="grid gap-x-4">
              <KV k="Display name" v={agent.displayName} />
              <KV k="Role" v={agent.role} />
              <KV k="Engine" v={engineLabel[agent.engine]} />
              <KV k="Manager" v={manager?.name ?? "—"} />
              <KV k="Specialization" v={agent.specialization} />
              <KV k="Concurrency" v={String(agent.concurrency)} />
              <KV k="Resource class" v={agent.resourceClass} />
              <KV k="Worktree" v={agent.worktreeRequired ? "required" : "shared"} />
              <KV k="Joined" v={agent.joinedAt} />
              <KV k="Usage today" v={`${agent.usagePct}% · $${agent.costTodayUsd.toFixed(2)}`} />
              <KV
                k="Current task"
                v={
                  current ? (
                    <Link
                      to="/tasks/$taskId"
                      params={{ taskId: current.id }}
                      className="text-primary hover:underline"
                    >
                      {current.id}
                    </Link>
                  ) : (
                    "—"
                  )
                }
              />
            </div>
          </Panel>

          <Panel title="Project familiarity">
            <ul className="space-y-2">
              {agent.familiarity.map((f) => (
                <li key={f.area}>
                  <div className="flex items-baseline justify-between text-[12px]">
                    <span>{f.area}</span>
                    <span className="mono-xs tabular-nums text-muted-foreground">{f.value}%</span>
                  </div>
                  <Meter
                    value={f.value}
                    tone={f.value > 70 ? "success" : f.value > 45 ? "info" : "muted"}
                    className="mt-1 h-1"
                  />
                </li>
              ))}
            </ul>
          </Panel>

          {reports.length > 0 && (
            <Panel title="Direct reports" dense>
              <ul className="divide-y divide-border">
                {reports.map((r) => (
                  <li key={r.id}>
                    <Link
                      to="/team/$agentId"
                      params={{ agentId: r.id }}
                      className="flex items-center gap-2 px-3 py-2 hover:bg-accent/40"
                    >
                      <Initials id={r.id} name={r.name} size={20} />
                      <span className="text-[12.5px] font-medium">{r.name}</span>
                      <span className="text-[11.5px] text-muted-foreground">{r.role}</span>
                      <Chip tone={agentStateTone[r.state]} className="ml-auto">
                        {r.state}
                      </Chip>
                    </Link>
                  </li>
                ))}
              </ul>
            </Panel>
          )}
        </div>

        <div className="space-y-3">
          <div className="grid gap-3 md:grid-cols-3">
            <Panel title="Skills">
              <div className="flex flex-wrap gap-1">
                {agent.skills.map((s) => (
                  <Chip key={s} tone="info">
                    {s}
                  </Chip>
                ))}
              </div>
            </Panel>
            <Panel title="Responsibilities">
              <ul className="space-y-1 text-[12px]">
                {agent.responsibilities.map((r) => (
                  <li key={r} className="flex gap-1.5">
                    <Dot tone="success" />
                    <span className="leading-tight">{r}</span>
                  </li>
                ))}
              </ul>
            </Panel>
            <Panel title="Cannot">
              <ul className="space-y-1 text-[12px]">
                {agent.restrictions.map((r) => (
                  <li key={r} className="flex gap-1.5 text-muted-foreground">
                    <UserX className="mt-0.5 size-3 shrink-0 text-destructive" />
                    <span className="leading-tight">{r}</span>
                  </li>
                ))}
              </ul>
            </Panel>
          </div>

          <Panel title="Skill packs attached" dense>
            <ul className="divide-y divide-border">
              {packs
                ?.filter((p) => p.usedBy.includes(agent.id))
                .map((p) => (
                  <li key={p.id} className="flex items-center gap-2 px-3 py-2">
                    <span className="text-[12.5px] font-medium">{p.name}</span>
                    <span className="mono-xs text-muted-foreground">v{p.version}</span>
                    <span className="ml-auto text-[11.5px] text-muted-foreground">
                      {p.description}
                    </span>
                  </li>
                ))}
            </ul>
          </Panel>

          <Panel title="Recent work" dense>
            <ul className="divide-y divide-border">
              {agent.recentTaskIds.map((id) => {
                const t = tasks?.find((x) => x.id === id);
                return (
                  <li key={id}>
                    <Link
                      to="/tasks/$taskId"
                      params={{ taskId: id }}
                      className="flex items-center gap-2 px-3 py-2 hover:bg-accent/40"
                    >
                      <span className="mono-xs text-muted-foreground">{id}</span>
                      <span className="min-w-0 flex-1 truncate text-[12px]">
                        {t?.title ?? "Archived work item"}
                      </span>
                      {t && <Chip tone="muted">{t.stage}</Chip>}
                    </Link>
                  </li>
                );
              })}
            </ul>
          </Panel>

          <Panel title="Relevant memory">
            {agentLessons.length === 0 ? (
              <p className="text-[12px] text-muted-foreground">No lessons attributed yet.</p>
            ) : (
              <ul className="space-y-2">
                {agentLessons.map((l) => (
                  <li key={l.id} className="rounded border border-border bg-surface px-2.5 py-2">
                    <div className="flex items-center gap-2">
                      <span className="mono-xs text-muted-foreground">{l.id}</span>
                      <Chip tone={l.confidence > 80 ? "success" : "warning"}>
                        {l.confidence}% confidence
                      </Chip>
                    </div>
                    <p className="mt-1 text-[12px] leading-relaxed">{l.body}</p>
                    <p className="mono-xs mt-1 text-muted-foreground">
                      sources: {l.sources.join(", ")}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </Panel>
        </div>
      </PageBody>
    </Page>
  );
}
