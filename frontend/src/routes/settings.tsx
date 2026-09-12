import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ProjectService, AgentService, PipelineService, KnowledgeService } from "@/services";
import { useProject } from "@/components/layout/project-context";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, Initials, KV, Panel } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
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
import { Save, Trash2 } from "lucide-react";

export const Route = createFileRoute("/settings")({
  head: () => ({
    meta: [
      { title: "Project Settings — Forge" },
      {
        name: "description",
        content:
          "General, repository, team, execution, knowledge provider, AI provider and integration settings.",
      },
      { property: "og:title", content: "Project Settings — Forge" },
      {
        property: "og:description",
        content: "Configure how the AI engineering team is allowed to operate.",
      },
    ],
  }),
  component: SettingsPage,
});

const SECTIONS = [
  "General",
  "Repository",
  "Team",
  "Execution",
  "Knowledge Providers",
  "AI Providers",
  "Integrations",
] as const;

function SettingsPage() {
  const { projectId } = useProject();
  const [section, setSection] = useState<(typeof SECTIONS)[number]>("General");
  const { data: project } = useQuery({
    queryKey: ["project", projectId],
    queryFn: () => ProjectService.get(projectId),
  });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: pipelines } = useQuery({ queryKey: ["pipelines"], queryFn: PipelineService.list });
  const { data: integrations } = useQuery({
    queryKey: ["integrations"],
    queryFn: ProjectService.integrations,
  });
  const { data: indexes } = useQuery({
    queryKey: ["codeIndexes"],
    queryFn: KnowledgeService.codeIndexes,
  });

  if (!project) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-72 w-full" />
      </PageBody>
    );
  }

  const byCategory = (cat: string) => integrations?.filter((i) => i.category === cat) ?? [];

  return (
    <Page>
      <PageHeader
        title="Project settings"
        subtitle={`${project.name} · ${project.repository}`}
        actions={
          <Button
            size="sm"
            className="h-7 text-[12px]"
            onClick={() => toast.success("Settings saved")}
          >
            <Save className="size-3.5" /> Save changes
          </Button>
        }
      />
      <PageBody className="grid gap-3 lg:grid-cols-[200px_1fr]">
        <nav className="panel h-fit p-1">
          {SECTIONS.map((s) => (
            <button
              key={s}
              onClick={() => setSection(s)}
              className={`block w-full rounded px-2 py-1.5 text-left text-[12.5px] transition-colors ${
                section === s ? "bg-accent font-medium" : "text-muted-foreground hover:bg-accent/50"
              }`}
            >
              {s}
            </button>
          ))}
        </nav>

        <div className="space-y-3">
          {section === "General" && (
            <>
              <Panel title="General">
                <div className="grid max-w-xl gap-3">
                  <Field label="Project name">
                    <Input defaultValue={project.name} className="h-7 text-[12px]" />
                  </Field>
                  <Field label="Project key">
                    <Input defaultValue={project.key} className="h-7 font-mono text-[12px]" />
                  </Field>
                  <Field label="Description">
                    <Textarea
                      defaultValue={project.description}
                      className="min-h-20 text-[12.5px]"
                    />
                  </Field>
                  <Field label="Environment">
                    <div className="flex items-center gap-2">
                      <Chip tone="muted">{project.environment}</Chip>
                      <Chip tone={project.environmentStatus === "healthy" ? "success" : "warning"}>
                        <Dot
                          tone={project.environmentStatus === "healthy" ? "success" : "warning"}
                        />
                        {project.environmentStatus}
                      </Chip>
                    </div>
                  </Field>
                </div>
              </Panel>
              <Panel title="Danger zone">
                <div className="flex items-center justify-between">
                  <p className="text-[12px] text-muted-foreground">
                    Archiving stops all scheduling and suspends every employee on this project.
                  </p>
                  <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button size="sm" variant="destructive" className="h-7 text-[12px]">
                        <Trash2 className="size-3.5" /> Archive project
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle className="text-[14px]">
                          Archive {project.name}?
                        </AlertDialogTitle>
                        <AlertDialogDescription className="text-[12.5px]">
                          Running work is aborted, workspaces are preserved read-only and no further
                          runs are admitted. This cannot be undone from the control room.
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel className="h-7 text-[12px]">Cancel</AlertDialogCancel>
                        <AlertDialogAction
                          className="h-7 text-[12px]"
                          onClick={() => toast.error("Project archived")}
                        >
                          Archive
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                </div>
              </Panel>
            </>
          )}

          {section === "Repository" && (
            <Panel title="Repository">
              <div className="grid max-w-xl gap-3">
                <Field label="Remote">
                  <Input defaultValue={project.repository} className="h-7 font-mono text-[12px]" />
                </Field>
                <Field label="Default branch">
                  <Input
                    defaultValue={project.defaultBranch}
                    className="h-7 font-mono text-[12px]"
                  />
                </Field>
                <Field label="Worktree root">
                  <Input
                    defaultValue="/var/forge/worktrees"
                    className="h-7 font-mono text-[12px]"
                  />
                </Field>
                <Toggle
                  label="Isolated worktree per work item"
                  hint="Required — workspaces belong to tasks, not to employees."
                  defaultChecked
                />
                <Toggle
                  label="Only the integration stage may write to the default branch"
                  hint="Enforced by the pipeline; individual employees can never merge."
                  defaultChecked
                />
              </div>
            </Panel>
          )}

          {section === "Team" && (
            <Panel title="Team defaults" dense>
              <ul className="divide-y divide-border">
                {agents?.map((a) => (
                  <li key={a.id} className="flex items-center gap-3 px-3 py-2">
                    <Initials id={a.id} name={a.name} size={20} />
                    <span className="text-[12.5px] font-medium">{a.name}</span>
                    <span className="text-[11.5px] text-muted-foreground">{a.role}</span>
                    <Chip tone="muted" className="ml-auto">
                      {engineLabel[a.engine]}
                    </Chip>
                    <Chip tone="info">concurrency {a.concurrency}</Chip>
                    <Chip tone="muted">{a.resourceClass}</Chip>
                  </li>
                ))}
              </ul>
            </Panel>
          )}

          {section === "Execution" && (
            <Panel title="Execution">
              <div className="grid max-w-xl gap-3">
                <Field label="Default pipeline">
                  <Input defaultValue={pipelines?.[0]?.name ?? ""} className="h-7 text-[12px]" />
                </Field>
                <Field label="Global max parallel runs">
                  <Input type="number" defaultValue={4} className="h-7 w-24 text-[12px]" />
                </Field>
                <Toggle
                  label="Auto-plan next wave when the current wave empties"
                  hint="Off keeps planning under human control."
                />
                <Toggle
                  label="Pause scheduling when verification fails 3 times in a row"
                  defaultChecked
                />
                <Toggle
                  label="Require human approval before ABI-affecting integration"
                  defaultChecked
                />
              </div>
            </Panel>
          )}

          {section === "Knowledge Providers" &&
            (indexes ?? []).map((ix) => (
              <Panel key={ix.id} title={`Code intelligence — ${ix.repo}`}>
                <div className="grid gap-x-6 md:grid-cols-2">
                  <KV k="Symbols" v={ix.symbols.toLocaleString()} />
                  <KV k="Providers" v={ix.providers.map((p) => p.name).join(", ")} />
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {ix.providers.map((p) => (
                    <Chip key={p.name} tone={p.connected ? "success" : "muted"}>
                      <Dot tone={p.connected ? "success" : "muted"} />
                      {p.name}
                    </Chip>
                  ))}
                </div>
              </Panel>
            ))}

          {section === "AI Providers" && (
            <IntegrationList title="AI providers" items={byCategory("ai")} />
          )}

          {section === "Integrations" && (
            <IntegrationList title="Integrations" items={integrations ?? []} />
          )}
        </div>
      </PageBody>
    </Page>
  );
}

function IntegrationList({
  title,
  items,
}: {
  title: string;
  items: { id: string; name: string; status: string; detail: string; category: string }[];
}) {
  return (
    <Panel title={title} dense>
      <ul className="divide-y divide-border">
        {items.map((i) => (
          <li key={i.id} className="flex items-center gap-3 px-3 py-2">
            <span className="text-[12.5px] font-medium">{i.name}</span>
            <span className="text-[11.5px] text-muted-foreground">{i.detail}</span>
            <Chip
              tone={
                i.status === "connected" ? "success" : i.status === "error" ? "danger" : "muted"
              }
              className="ml-auto"
            >
              <Dot
                tone={
                  i.status === "connected" ? "success" : i.status === "error" ? "danger" : "muted"
                }
              />
              {i.status === "not_configured" ? "not configured" : i.status}
            </Chip>
            <Button
              size="sm"
              variant="outline"
              className="h-6 text-[11.5px]"
              onClick={() =>
                toast(
                  `${i.name} ${i.status === "connected" ? "reconfigured" : "connection started"}`,
                )
              }
            >
              {i.status === "connected" ? "Configure" : "Connect"}
            </Button>
          </li>
        ))}
      </ul>
    </Panel>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1.5">
      <Label className="text-[11.5px]">{label}</Label>
      {children}
    </div>
  );
}

function Toggle({
  label,
  hint,
  defaultChecked = false,
}: {
  label: string;
  hint?: string | undefined;
  defaultChecked?: boolean | undefined;
}) {
  return (
    <label className="flex items-start justify-between gap-3 rounded-md border border-border px-2.5 py-2">
      <span>
        <span className="block text-[12.5px]">{label}</span>
        {hint && <span className="block text-[11px] text-muted-foreground">{hint}</span>}
      </span>
      <Switch
        defaultChecked={defaultChecked}
        onCheckedChange={(v) => toast(`${label}: ${v ? "on" : "off"}`)}
      />
    </label>
  );
}
