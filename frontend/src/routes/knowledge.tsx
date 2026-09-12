import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { KnowledgeService, AgentService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, Initials, KV, Meter, Panel } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { relTime } from "@/lib/format";
import { BookLock, GitBranch, GraduationCap, Package, Database, Plus } from "lucide-react";

export const Route = createFileRoute("/knowledge")({
  head: () => ({
    meta: [
      { title: "Knowledge — Forge" },
      {
        name: "description",
        content:
          "Structured project knowledge: policies, decisions, extracted lessons, skill packs and code intelligence.",
      },
      { property: "og:title", content: "Knowledge — Forge" },
      {
        property: "og:description",
        content: "Never one giant memory file — knowledge is categorised and cited.",
      },
    ],
  }),
  component: KnowledgePage,
});

const TABS = [
  { id: "policies", label: "Policies", icon: BookLock },
  { id: "decisions", label: "Decisions", icon: GitBranch },
  { id: "lessons", label: "Lessons", icon: GraduationCap },
  { id: "skills", label: "Skills", icon: Package },
  { id: "code", label: "Code Intelligence", icon: Database },
] as const;

function KnowledgePage() {
  const [tab, setTab] = useState<(typeof TABS)[number]["id"]>("policies");
  const { data: policies } = useQuery({
    queryKey: ["policies"],
    queryFn: KnowledgeService.policies,
  });
  const { data: decisions } = useQuery({
    queryKey: ["decisions"],
    queryFn: KnowledgeService.decisions,
  });
  const { data: lessons } = useQuery({ queryKey: ["lessons"], queryFn: KnowledgeService.lessons });
  const { data: packs } = useQuery({
    queryKey: ["skillPacks"],
    queryFn: KnowledgeService.skillPacks,
  });
  const { data: indexes } = useQuery({
    queryKey: ["codeIndexes"],
    queryFn: KnowledgeService.codeIndexes,
  });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });

  return (
    <Page>
      <PageHeader
        title="Knowledge"
        subtitle="Everything the team is allowed to rely on, split by authority and provenance"
        actions={
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-[12px]"
            onClick={() => toast("New knowledge entry")}
          >
            <Plus className="size-3.5" /> New entry
          </Button>
        }
        tabs={
          <div className="flex flex-wrap gap-1">
            {TABS.map((t) => (
              <Button
                key={t.id}
                size="sm"
                variant={tab === t.id ? "secondary" : "ghost"}
                className="h-6 text-[11.5px]"
                onClick={() => setTab(t.id)}
              >
                <t.icon className="size-3" /> {t.label}
              </Button>
            ))}
          </div>
        }
      />
      <PageBody className="space-y-2">
        {tab === "policies" &&
          (!policies ? (
            <Skeleton className="h-40 w-full" />
          ) : (
            policies.map((p) => (
              <Panel key={p.id}>
                <div className="flex items-center gap-2">
                  <span className="mono-xs text-muted-foreground">{p.id}</span>
                  <span className="text-[12.5px] font-semibold">{p.title}</span>
                  <Chip
                    tone={
                      p.status === "active" ? "success" : p.status === "draft" ? "warning" : "muted"
                    }
                    className="ml-auto"
                  >
                    {p.status}
                  </Chip>
                  <Chip tone="info">authority: {p.authority}</Chip>
                </div>
                <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{p.body}</p>
              </Panel>
            ))
          ))}

        {tab === "decisions" &&
          (!decisions ? (
            <Skeleton className="h-40 w-full" />
          ) : (
            decisions.map((d) => (
              <Panel key={d.id}>
                <div className="flex items-center gap-2">
                  <span className="mono-xs text-muted-foreground">{d.id}</span>
                  <span className="text-[12.5px] font-semibold">{d.title}</span>
                  <Chip
                    tone={
                      d.status === "accepted"
                        ? "success"
                        : d.status === "proposed"
                          ? "warning"
                          : "muted"
                    }
                    className="ml-auto"
                  >
                    {d.status}
                  </Chip>
                  {d.supersedes && <Chip tone="danger">supersedes {d.supersedes}</Chip>}
                </div>
                <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{d.body}</p>
                <p className="mono-xs mt-1 text-muted-foreground">decided {relTime(d.decidedAt)}</p>
              </Panel>
            ))
          ))}

        {tab === "lessons" &&
          (!lessons ? (
            <Skeleton className="h-40 w-full" />
          ) : (
            lessons.map((l) => (
              <Panel key={l.id}>
                <div className="flex items-center gap-2">
                  <span className="mono-xs text-muted-foreground">{l.id}</span>
                  <Chip tone="muted">auto-extracted</Chip>
                  <div className="ml-auto flex items-center gap-1.5">
                    <Meter
                      value={l.confidence}
                      tone={l.confidence > 80 ? "success" : "warning"}
                      className="w-20"
                    />
                    <span className="mono-xs">{l.confidence}%</span>
                  </div>
                </div>
                <p className="mt-1 text-[12.5px] leading-relaxed">{l.body}</p>
                <p className="mono-xs mt-1 text-muted-foreground">
                  sources: {l.sources.join(", ")}
                </p>
              </Panel>
            ))
          ))}

        {tab === "skills" &&
          (!packs ? (
            <Skeleton className="h-40 w-full" />
          ) : (
            <Panel title="Skill packs" dense>
              <ul className="divide-y divide-border">
                {packs.map((p) => (
                  <li key={p.id} className="flex items-center gap-3 px-3 py-2">
                    <span className="text-[12.5px] font-medium">{p.name}</span>
                    <span className="mono-xs text-muted-foreground">v{p.version}</span>
                    <span className="min-w-0 flex-1 truncate text-[11.5px] text-muted-foreground">
                      {p.description}
                    </span>
                    <span className="flex items-center gap-1">
                      {p.usedBy.map((id) => {
                        const a = agents?.find((x) => x.id === id);
                        return a ? <Initials key={id} id={a.id} name={a.name} size={18} /> : null;
                      })}
                    </span>
                  </li>
                ))}
              </ul>
            </Panel>
          ))}

        {tab === "code" &&
          (!indexes ? (
            <Skeleton className="h-40 w-full" />
          ) : (
            indexes.map((ix) => (
              <Panel key={ix.id} title={ix.repo}>
                <div className="grid gap-x-6 md:grid-cols-2">
                  <KV k="Symbols indexed" v={ix.symbols.toLocaleString()} />
                  <KV k="Last indexed" v={relTime(ix.indexedAt)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {ix.providers.map((pr) => (
                    <Chip key={pr.name} tone={pr.connected ? "success" : "muted"}>
                      <Dot tone={pr.connected ? "success" : "muted"} />
                      {pr.name}: {pr.connected ? "connected" : "not connected"}
                    </Chip>
                  ))}
                </div>
              </Panel>
            ))
          ))}
      </PageBody>
    </Page>
  );
}
