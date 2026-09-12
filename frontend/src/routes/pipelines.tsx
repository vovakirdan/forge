import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { PipelineService } from "@/services";
import type { PipelineStageConfig } from "@/data/types";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, KV, Panel, StageBadge } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { ArrowDown, Plus, Save, ShieldCheck } from "lucide-react";

export const Route = createFileRoute("/pipelines")({
  head: () => ({
    meta: [
      { title: "Pipeline Designer — Forge" },
      {
        name: "description",
        content:
          "Visual state machine for engineering work: executors, checks, attempts, resource classes and failure transitions.",
      },
      { property: "og:title", content: "Pipeline Designer — Forge" },
      {
        property: "og:description",
        content: "Pipelines enforce verification and review, not agent goodwill.",
      },
    ],
  }),
  component: PipelinesPage,
});

function PipelinesPage() {
  const { data: pipelines } = useQuery({ queryKey: ["pipelines"], queryFn: PipelineService.list });
  const { data: available } = useQuery({
    queryKey: ["availableStages"],
    queryFn: PipelineService.availableStages,
  });
  const [pipelineId, setPipelineId] = useState<string | null>(null);
  const [stageId, setStageId] = useState<string | null>(null);

  if (!pipelines) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-80 w-full" />
      </PageBody>
    );
  }

  const pipeline = pipelines.find((p) => p.id === pipelineId) ?? pipelines[0];
  if (!pipeline) return null;
  const stage =
    pipeline.stages.find((s) => s.id === stageId) ?? pipeline.stages[1] ?? pipeline.stages[0];
  if (!stage) return null;

  return (
    <Page>
      <PageHeader
        title="Pipeline Designer"
        subtitle="A work item moves through these stages. Failures transition backwards — they never spawn new tasks."
        actions={
          <div className="flex items-center gap-1.5">
            <Select
              value={pipeline.id}
              onValueChange={(v) => {
                setPipelineId(v);
                setStageId(null);
              }}
            >
              <SelectTrigger className="h-7 w-56 text-[12px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {pipelines.map((p) => (
                  <SelectItem key={p.id} value={p.id} className="text-[12.5px]">
                    {p.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              size="sm"
              variant="outline"
              className="h-7 text-[12px]"
              onClick={() => toast("New pipeline draft created")}
            >
              <Plus className="size-3.5" /> New pipeline
            </Button>
            <Button
              size="sm"
              className="h-7 text-[12px]"
              onClick={() =>
                toast.success("Pipeline saved", {
                  description: `${pipeline.tasksUsing} work items follow this definition.`,
                })
              }
            >
              <Save className="size-3.5" /> Save
            </Button>
          </div>
        }
      />
      <PageBody className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="space-y-3">
          <Panel
            title={pipeline.name}
            action={<Chip tone="muted">{pipeline.tasksUsing} work items using</Chip>}
          >
            <p className="text-[12px] text-muted-foreground">{pipeline.description}</p>
            <p className="mono-xs mt-1 text-muted-foreground">applies to: {pipeline.appliesTo}</p>
          </Panel>

          <Panel title="State machine" dense>
            <div className="space-y-0 p-3">
              {pipeline.stages.map((s, i) => (
                <div key={s.id}>
                  <button
                    onClick={() => setStageId(s.id)}
                    className={cn(
                      "flex w-full items-start gap-3 rounded-md border px-3 py-2.5 text-left transition-colors",
                      s.id === stage.id
                        ? "border-primary bg-primary/5"
                        : "border-border bg-surface hover:border-border-strong",
                    )}
                  >
                    <StageBadge stage={s.stage} />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="text-[12.5px] font-semibold">{s.name}</span>
                        <Chip
                          tone={
                            s.executorKind === "system"
                              ? "info"
                              : s.executorKind === "human"
                                ? "warning"
                                : "primary"
                          }
                        >
                          {s.executorKind === "role" ? `role: ${s.executor}` : s.executor}
                        </Chip>
                        {s.resourceClass && (
                          <Chip tone={s.resourceClass === "heavy" ? "danger" : "muted"}>
                            {s.resourceClass}
                          </Chip>
                        )}
                        {s.concurrency !== undefined && (
                          <Chip tone="muted">concurrency {s.concurrency}</Chip>
                        )}
                      </div>
                      <div className="mt-1 flex flex-wrap gap-x-4 gap-y-0.5 text-[11.5px] text-muted-foreground">
                        {s.maxAttempts && <span>max attempts {s.maxAttempts}</span>}
                        {s.workspace && <span>workspace: {s.workspace}</span>}
                        {s.profile && <span>profile: {s.profile}</span>}
                        {s.checks && <span>{s.checks.length} required checks</span>}
                        {s.failureTransition && (
                          <span className="text-destructive">
                            on failure → {s.failureTransition}
                          </span>
                        )}
                      </div>
                    </div>
                  </button>
                  {i < pipeline.stages.length - 1 && (
                    <div className="flex items-center gap-2 py-1 pl-6 text-muted-foreground">
                      <ArrowDown className="size-3.5" />
                      <span className="mono-xs">on success</span>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </Panel>

          <Panel title="Add stage">
            <div className="flex flex-wrap gap-1.5">
              {available?.map((s) => (
                <Button
                  key={s}
                  size="sm"
                  variant="outline"
                  className="h-7 text-[12px]"
                  onClick={() =>
                    toast(`${s} stage added`, {
                      description: "Configure its executor before saving.",
                    })
                  }
                >
                  <Plus className="size-3.5" /> {s}
                </Button>
              ))}
            </div>
          </Panel>
        </div>

        <Panel title="Stage configuration">
          <StageConfig stage={stage} />
        </Panel>
      </PageBody>
    </Page>
  );
}

function StageConfig({ stage }: { stage: PipelineStageConfig }) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <StageBadge stage={stage.stage} />
        <span className="text-[13px] font-semibold">{stage.name}</span>
      </div>

      <div className="space-y-1.5">
        <Label className="text-[11.5px]">Executor</Label>
        <Select
          defaultValue={stage.executorKind}
          onValueChange={(v) => toast(`Executor set to ${v}`)}
        >
          <SelectTrigger className="h-7 text-[12px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="role" className="text-[12.5px]">
              Role
            </SelectItem>
            <SelectItem value="system" className="text-[12.5px]">
              System
            </SelectItem>
            <SelectItem value="human" className="text-[12.5px]">
              Human
            </SelectItem>
          </SelectContent>
        </Select>
        <Input defaultValue={stage.executor} className="h-7 text-[12px]" />
      </div>

      {stage.maxAttempts !== undefined && (
        <div className="space-y-1.5">
          <Label className="text-[11.5px]">Max attempts</Label>
          <Input type="number" defaultValue={stage.maxAttempts} className="h-7 w-24 text-[12px]" />
        </div>
      )}

      {stage.workspace && (
        <div className="space-y-1.5">
          <Label className="text-[11.5px]">Workspace</Label>
          <Input defaultValue={stage.workspace} className="h-7 text-[12px]" />
        </div>
      )}

      {stage.profile && (
        <div className="space-y-1.5">
          <Label className="text-[11.5px]">Verification profile</Label>
          <Input defaultValue={stage.profile} className="h-7 font-mono text-[12px]" />
        </div>
      )}

      {stage.checks && (
        <div>
          <p className="section-label mb-1.5">Required checks</p>
          <ul className="space-y-1">
            {stage.checks.map((c) => (
              <li
                key={c}
                className="flex items-center justify-between rounded border border-border bg-surface px-2 py-1 text-[12px]"
              >
                <span className="flex items-center gap-1.5">
                  <ShieldCheck className="size-3 text-success" />
                  {c}
                </span>
                <Switch
                  defaultChecked
                  onCheckedChange={(v) => toast(`${c} ${v ? "required" : "optional"}`)}
                />
              </li>
            ))}
          </ul>
        </div>
      )}

      {stage.actions && (
        <div>
          <p className="section-label mb-1.5">Actions</p>
          <ol className="space-y-1">
            {stage.actions.map((a, i) => (
              <li
                key={a}
                className="flex items-center gap-2 rounded border border-border bg-surface px-2 py-1 text-[12px]"
              >
                <span className="mono-xs text-muted-foreground">{i + 1}</span>
                {a}
              </li>
            ))}
          </ol>
        </div>
      )}

      {stage.rules && (
        <div>
          <p className="section-label mb-1.5">Rules</p>
          <ul className="space-y-1">
            {stage.rules.map((r) => (
              <li key={r} className="flex items-start gap-1.5 text-[12px]">
                <Dot tone="warning" />
                <span className="leading-tight">{r}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="grid gap-x-4 border-t border-border pt-2">
        <KV k="Resource class" v={stage.resourceClass ?? "—"} />
        <KV k="Concurrency" v={stage.concurrency !== undefined ? String(stage.concurrency) : "—"} />
        <KV k="On failure" v={stage.failureTransition ?? "terminal"} />
      </div>

      <p className="text-[11px] leading-relaxed text-muted-foreground">
        A failure transition returns the same work item to an earlier stage and increments its
        attempt counter. No duplicate card is ever created.
      </p>
    </div>
  );
}
