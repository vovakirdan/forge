import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { ResourceService, RunService, TaskService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, KV, Metric, Meter, Panel } from "@/components/common/Bits";
import { Skeleton } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";
import { durationSec } from "@/lib/format";
import { Cpu, HardDrive, Layers, Gauge } from "lucide-react";

export const Route = createFileRoute("/resources")({
  head: () => ({
    meta: [
      { title: "Resources & Scheduling — Forge" },
      {
        name: "description",
        content:
          "Host limits, resource pools and queues. Backlog size and runtime concurrency are separate concepts.",
      },
      { property: "og:title", content: "Resources & Scheduling — Forge" },
      {
        property: "og:description",
        content: "A hundred tasks can exist while only a few execute.",
      },
    ],
  }),
  component: ResourcesPage,
});

function ResourcesPage() {
  const { data: host } = useQuery({ queryKey: ["host"], queryFn: ResourceService.host });
  const { data: pools } = useQuery({ queryKey: ["pools"], queryFn: ResourceService.pools });
  const { data: runs } = useQuery({ queryKey: ["runs"], queryFn: RunService.list });
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });

  if (!host || !pools) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-64 w-full" />
      </PageBody>
    );
  }

  const running = runs?.filter((r) => r.status === "running") ?? [];

  return (
    <Page>
      <PageHeader
        title="Resources & Scheduling"
        subtitle={`${tasks?.length ?? 0} work items exist · ${running.length} are actually executing right now`}
        actions={
          <Chip tone="warning">
            <Dot tone="warning" pulse /> Heavy pool saturated
          </Chip>
        }
      />
      <PageBody className="space-y-3">
        <div className="panel grid grid-cols-2 divide-x divide-border md:grid-cols-3 xl:grid-cols-6">
          <Metric
            label="CPU"
            value={`${host.cpuThreadsUsed} / ${host.cpuThreadsTotal} threads`}
            hint={`${host.cpuPct}% utilised`}
          />
          <Metric
            label="RAM"
            value={`${host.ramUsedGb} / ${host.ramTotalGb} GB`}
            hint="agent workspaces + test runners"
          />
          <Metric
            label="Disk IO"
            value={host.diskIo}
            tone="warning"
            hint="verification workload dominant"
          />
          <Metric
            label="Active runs"
            value={`${host.activeRuns} / ${host.maxRuns}`}
            tone="running"
          />
          <Metric
            label="Heavy verification"
            value={`${host.heavyInUse} / ${host.heavyCapacity}`}
            tone="danger"
            hint="exclusive execution"
          />
          <Metric
            label="Backlog"
            value={`${tasks?.length ?? 0} items`}
            hint="independent of concurrency"
          />
        </div>

        <div className="grid gap-3 xl:grid-cols-[1fr_360px]">
          <Panel title="Resource pools" dense>
            <table className="w-full text-[12px]">
              <thead className="border-b border-border text-left">
                <tr className="section-label">
                  <th className="px-3 py-1.5 font-semibold">Pool</th>
                  <th className="px-2 py-1.5 font-semibold">Usage</th>
                  <th className="px-2 py-1.5 font-semibold">Concurrency</th>
                  <th className="px-2 py-1.5 font-semibold">Priority</th>
                  <th className="px-2 py-1.5 font-semibold">CPU w.</th>
                  <th className="px-2 py-1.5 font-semibold">Memory</th>
                  <th className="px-2 py-1.5 font-semibold">IO w.</th>
                  <th className="px-2 py-1.5 font-semibold">Exclusive</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {pools.map((p) => (
                  <tr key={p.id} className="hover:bg-accent/40">
                    <td className="px-3 py-2 font-medium">
                      <span className="flex items-center gap-1.5">
                        <Layers className="size-3 text-muted-foreground" />
                        {p.name}
                      </span>
                    </td>
                    <td className="w-32 px-2 py-2">
                      <div className="flex items-center gap-1.5">
                        <Meter
                          value={(p.inUse / p.capacity) * 100}
                          tone={p.inUse >= p.capacity ? "danger" : "primary"}
                          className="w-16"
                        />
                        <span className="mono-xs tabular-nums">
                          {p.inUse}/{p.capacity}
                        </span>
                      </div>
                    </td>
                    <td className="w-40 px-2 py-2">
                      <Slider
                        defaultValue={[p.capacity]}
                        min={1}
                        max={6}
                        step={1}
                        className="w-28"
                        onValueCommit={(v) => toast(`${p.name} concurrency set to ${v[0]}`)}
                      />
                    </td>
                    <td className="px-2 py-2">
                      <Chip
                        tone={
                          p.priority === "high"
                            ? "warning"
                            : p.priority === "background"
                              ? "muted"
                              : "info"
                        }
                      >
                        {p.priority}
                      </Chip>
                    </td>
                    <td className="mono-xs px-2 py-2">{p.cpuWeight}</td>
                    <td className="mono-xs px-2 py-2">{p.memoryBudgetGb} GB</td>
                    <td className="mono-xs px-2 py-2">{p.ioWeight}</td>
                    <td className="px-2 py-2">
                      <Switch
                        defaultChecked={p.exclusive}
                        onCheckedChange={(v) =>
                          toast(`${p.name} exclusive execution ${v ? "on" : "off"}`)
                        }
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Panel>

          <div className="space-y-3">
            <Panel title="Queues">
              <ul className="space-y-2">
                {host.queues.map((q) => (
                  <li key={q.name}>
                    <div className="flex items-baseline justify-between">
                      <span className="text-[12px] font-medium">{q.name}</span>
                      <span className="mono-xs tabular-nums text-muted-foreground">
                        {q.waiting} waiting
                      </span>
                    </div>
                    <Meter
                      value={Math.min(100, q.waiting * 20)}
                      tone={q.waiting > 3 ? "danger" : q.waiting > 0 ? "warning" : "muted"}
                      className="mt-1 h-1"
                    />
                  </li>
                ))}
              </ul>
              <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
                Queued work does not consume host resources. The scheduler admits runs only when a
                pool slot, CPU weight and memory budget are all available.
              </p>
            </Panel>

            <Panel title="Executing now" dense>
              <ul className="divide-y divide-border">
                {running.map((r) => (
                  <li key={r.id}>
                    <Link
                      to="/runs/$runId"
                      params={{ runId: r.id }}
                      className="block px-3 py-2 hover:bg-accent/40"
                    >
                      <div className="flex items-center gap-2">
                        <Dot tone="running" pulse />
                        <span className="mono-xs text-muted-foreground">{r.id}</span>
                        <Chip tone="muted">{r.resourceClass}</Chip>
                        <span className="mono-xs ml-auto">{durationSec(r.durationSec)}</span>
                      </div>
                      <p className="mt-0.5 truncate text-[12px]">{r.reason}</p>
                      <div className="mt-1 grid grid-cols-2 gap-x-3">
                        <KV k="CPU" v={`${r.cpuPct}%`} />
                        <KV k="RAM" v={`${r.ramGb} GB`} />
                      </div>
                    </Link>
                  </li>
                ))}
              </ul>
            </Panel>

            <Panel title="Host controls">
              <div className="space-y-3">
                <div>
                  <div className="flex items-center justify-between text-[12px]">
                    <span className="flex items-center gap-1.5">
                      <Cpu className="size-3 text-muted-foreground" /> Max parallel runs
                    </span>
                    <span className="mono-xs">{host.maxRuns}</span>
                  </div>
                  <Slider
                    defaultValue={[host.maxRuns]}
                    min={1}
                    max={8}
                    step={1}
                    className="mt-2"
                    onValueCommit={(v) => toast(`Max parallel runs set to ${v[0]}`)}
                  />
                </div>
                <div>
                  <div className="flex items-center justify-between text-[12px]">
                    <span className="flex items-center gap-1.5">
                      <HardDrive className="size-3 text-muted-foreground" /> Memory ceiling
                    </span>
                    <span className="mono-xs">{host.ramTotalGb} GB</span>
                  </div>
                  <Slider
                    defaultValue={[host.ramTotalGb]}
                    min={8}
                    max={64}
                    step={4}
                    className="mt-2"
                    onValueCommit={(v) => toast(`Memory ceiling set to ${v[0]} GB`)}
                  />
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 w-full text-[12px]"
                  onClick={() =>
                    toast.success("Scheduler drained", {
                      description: "Running work finishes; nothing new is admitted.",
                    })
                  }
                >
                  <Gauge className="size-3.5" /> Drain scheduler
                </Button>
              </div>
            </Panel>
          </div>
        </div>
      </PageBody>
    </Page>
  );
}
