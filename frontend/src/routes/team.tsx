import { createFileRoute, Link, Outlet, useMatchRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { AgentService, TaskService, RunService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, Initials, Meter, Panel, agentStateTone } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { HireWizard } from "@/components/team/HireWizard";
import { engineLabel } from "@/lib/format";
import { UserPlus, ChevronRight } from "lucide-react";

export const Route = createFileRoute("/team")({
  head: () => ({
    meta: [
      { title: "Team — Forge" },
      {
        name: "description",
        content:
          "The AI engineering org: roles, engines, current tasks, responsibilities and usage per employee.",
      },
      { property: "og:title", content: "Team — Forge" },
      {
        property: "og:description",
        content: "Persistent AI employees with defined scope and limits.",
      },
    ],
  }),
  component: TeamPage,
});

function TeamPage() {
  const matchRoute = useMatchRoute();
  const isProfile = !!matchRoute({ to: "/team/$agentId", fuzzy: true });
  if (isProfile) return <Outlet />;
  return <TeamList />;
}

function TeamList() {
  const [hiring, setHiring] = useState(false);
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });
  const { data: runs } = useQuery({ queryKey: ["runs"], queryFn: RunService.list });

  const lead = agents?.find((a) => !a.managerId);

  return (
    <Page>
      <PageHeader
        title="Team"
        subtitle={`${agents?.length ?? 0} persistent employees · scope and permissions enforced by the pipeline`}
        actions={
          <Button size="sm" className="h-7 text-[12px]" onClick={() => setHiring(true)}>
            <UserPlus className="size-3.5" /> Hire employee
          </Button>
        }
      />
      <PageBody className="space-y-3">
        {!agents ? (
          <Skeleton className="h-64 w-full" />
        ) : (
          <>
            <Panel title="Organisation">
              <div className="flex flex-wrap items-start gap-2 text-[12px]">
                {lead && (
                  <div className="rounded-md border border-primary/30 bg-primary/5 px-2.5 py-1.5">
                    <div className="flex items-center gap-1.5">
                      <Initials id={lead.id} name={lead.name} size={20} />
                      <span className="font-medium">{lead.name}</span>
                      <Chip tone="primary">{lead.role}</Chip>
                    </div>
                  </div>
                )}
                <ChevronRight className="mt-2 size-3.5 text-muted-foreground" />
                <div className="flex flex-wrap gap-2">
                  {agents
                    .filter((a) => a.managerId === lead?.id)
                    .map((a) => (
                      <div key={a.id} className="rounded-md border border-border px-2.5 py-1.5">
                        <div className="flex items-center gap-1.5">
                          <Initials id={a.id} name={a.name} size={20} />
                          <span className="font-medium">{a.name}</span>
                          <Chip tone={agentStateTone[a.state]}>
                            <Dot tone={agentStateTone[a.state]} pulse={a.state === "working"} />
                            {a.state}
                          </Chip>
                        </div>
                      </div>
                    ))}
                </div>
              </div>
            </Panel>

            <Panel title="Employees" dense>
              <table className="w-full text-[12px]">
                <thead className="border-b border-border text-left">
                  <tr className="section-label">
                    <th className="px-3 py-1.5 font-semibold">Employee</th>
                    <th className="px-2 py-1.5 font-semibold">Role</th>
                    <th className="px-2 py-1.5 font-semibold">Engine</th>
                    <th className="px-2 py-1.5 font-semibold">State</th>
                    <th className="px-2 py-1.5 font-semibold">Current task</th>
                    <th className="px-2 py-1.5 font-semibold">Active run</th>
                    <th className="px-2 py-1.5 font-semibold">Specialization</th>
                    <th className="px-2 py-1.5 font-semibold">Usage today</th>
                    <th className="px-2 py-1.5 font-semibold">Manager</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {agents.map((a) => {
                    const task = tasks?.find((t) => t.id === a.currentTaskId);
                    const run = runs?.find((r) => r.id === a.currentRunId);
                    const manager = agents.find((m) => m.id === a.managerId);
                    return (
                      <tr key={a.id} className="hover:bg-accent/40">
                        <td className="px-3 py-2">
                          <Link
                            to="/team/$agentId"
                            params={{ agentId: a.id }}
                            className="flex items-center gap-2 hover:underline"
                          >
                            <Initials id={a.id} name={a.name} size={22} />
                            <span className="font-medium">{a.name}</span>
                          </Link>
                        </td>
                        <td className="px-2 py-2 text-muted-foreground">{a.role}</td>
                        <td className="px-2 py-2">
                          <Chip tone="muted">{engineLabel[a.engine]}</Chip>
                        </td>
                        <td className="px-2 py-2">
                          <Chip tone={agentStateTone[a.state]}>
                            <Dot tone={agentStateTone[a.state]} pulse={a.state === "working"} />
                            {a.state}
                          </Chip>
                        </td>
                        <td className="px-2 py-2">
                          {task ? (
                            <Link
                              to="/tasks/$taskId"
                              params={{ taskId: task.id }}
                              className="mono-xs text-primary hover:underline"
                            >
                              {task.id}
                            </Link>
                          ) : (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </td>
                        <td className="px-2 py-2">
                          {run ? (
                            <Link
                              to="/runs/$runId"
                              params={{ runId: run.id }}
                              className="mono-xs text-primary hover:underline"
                            >
                              {run.id}
                            </Link>
                          ) : (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </td>
                        <td className="px-2 py-2 text-muted-foreground">{a.specialization}</td>
                        <td className="w-32 px-2 py-2">
                          <div className="flex items-center gap-1.5">
                            <Meter
                              value={a.usagePct}
                              tone={a.usagePct > 50 ? "warning" : "primary"}
                              className="w-14"
                            />
                            <span className="mono-xs tabular-nums">{a.usagePct}%</span>
                          </div>
                        </td>
                        <td className="px-2 py-2 text-muted-foreground">{manager?.name ?? "—"}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </Panel>
          </>
        )}
      </PageBody>
      <HireWizard open={hiring} onOpenChange={setHiring} />
    </Page>
  );
}
