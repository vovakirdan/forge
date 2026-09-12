import { Link, useRouterState } from "@tanstack/react-router";
import {
  Activity,
  BookOpen,
  Bot,
  Cpu,
  GitBranch,
  LayoutDashboard,
  MessageSquare,
  Settings,
  Sparkles,
  Target,
  Columns3,
  ChevronsUpDown,
  Check,
} from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { ProjectService, ResourceService, RunService } from "@/services";
import { cn } from "@/lib/utils";
import { Dot, Meter } from "@/components/common/Bits";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useProject } from "@/components/layout/project-context";
import { APP_NAME } from "@/config/app";

const nav = [
  { to: "/", label: "Overview", icon: LayoutDashboard },
  { to: "/board", label: "Board", icon: Columns3 },
  { to: "/goals", label: "Goals", icon: Target },
  { to: "/team", label: "Team", icon: Bot },
  { to: "/pipelines", label: "Pipelines", icon: GitBranch },
  { to: "/chat", label: "Chat", icon: MessageSquare },
  { to: "/activity", label: "Activity", icon: Activity },
  { to: "/knowledge", label: "Knowledge", icon: BookOpen },
  { to: "/resources", label: "Resources", icon: Cpu },
  { to: "/settings", label: "Settings", icon: Settings },
] as const;

export function AppSidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { projectId, setProjectId } = useProject();
  const { data: projects } = useQuery({ queryKey: ["projects"], queryFn: ProjectService.list });
  const { data: host } = useQuery({ queryKey: ["host"], queryFn: ResourceService.host });
  const { data: activeRuns } = useQuery({
    queryKey: ["runs", "active"],
    queryFn: RunService.active,
  });

  const project = projects?.find((p) => p.id === projectId) ?? projects?.[0];

  return (
    <aside className="flex w-[216px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar">
      <div className="flex h-11 items-center gap-2 border-b border-sidebar-border px-3">
        <span className="grid size-5 place-items-center rounded bg-primary text-primary-foreground">
          <Sparkles className="size-3" />
        </span>
        <span className="text-[13px] font-semibold tracking-tight">{APP_NAME}</span>
      </div>

      <DropdownMenu>
        <DropdownMenuTrigger className="flex items-center gap-2 border-b border-sidebar-border px-3 py-2 text-left transition-colors hover:bg-sidebar-accent">
          <div className="min-w-0 flex-1">
            <div className="truncate text-[12.5px] font-medium">{project?.name ?? "—"}</div>
            <div className="mono-xs truncate text-muted-foreground">{project?.repository}</div>
          </div>
          <ChevronsUpDown className="size-3.5 text-muted-foreground" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-64">
          <DropdownMenuLabel className="text-[11px]">Projects</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {projects?.map((p) => (
            <DropdownMenuItem key={p.id} onSelect={() => setProjectId(p.id)} className="gap-2">
              <Dot tone={p.environmentStatus === "healthy" ? "success" : "warning"} />
              <span className="flex-1 truncate">{p.name}</span>
              {p.id === project?.id && <Check className="size-3.5" />}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <nav className="flex-1 overflow-auto p-2">
        {nav.map(({ to, label, icon: Icon }) => {
          const active = to === "/" ? pathname === "/" : pathname.startsWith(to);
          return (
            <Link
              key={to}
              to={to}
              className={cn(
                "mb-0.5 flex items-center gap-2 rounded-md px-2 py-1.5 text-[12.5px] font-medium transition-colors",
                active
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "text-sidebar-foreground hover:bg-sidebar-accent/60",
              )}
            >
              <Icon className={cn("size-3.5", active ? "text-primary" : "text-muted-foreground")} />
              {label}
            </Link>
          );
        })}
      </nav>

      <div className="space-y-2 border-t border-sidebar-border px-3 py-2.5">
        <div className="flex items-center gap-1.5">
          <Dot tone="success" pulse />
          <span className="text-[11px] font-medium">Scheduler healthy</span>
        </div>
        <div className="space-y-1.5">
          <SysRow label="CPU" value={`${host?.cpuPct ?? 0}%`} pct={host?.cpuPct ?? 0} />
          <SysRow
            label="RAM"
            value={`${host?.ramUsedGb ?? 0} / ${host?.ramTotalGb ?? 0} GB`}
            pct={((host?.ramUsedGb ?? 0) / (host?.ramTotalGb ?? 1)) * 100}
          />
          <SysRow
            label="Active runs"
            value={`${activeRuns?.length ?? 0} / ${host?.maxRuns ?? 0}`}
            pct={((activeRuns?.length ?? 0) / (host?.maxRuns ?? 1)) * 100}
          />
          <SysRow
            label="Heavy verification"
            value={`${host?.heavyInUse ?? 0} / ${host?.heavyCapacity ?? 0}`}
            pct={((host?.heavyInUse ?? 0) / (host?.heavyCapacity ?? 1)) * 100}
            tone="warning"
          />
        </div>
      </div>
    </aside>
  );
}

function SysRow({
  label,
  value,
  pct,
  tone,
}: {
  label: string;
  value: string;
  pct: number;
  tone?: "warning";
}) {
  return (
    <div>
      <div className="flex items-baseline justify-between">
        <span className="text-[10.5px] text-muted-foreground">{label}</span>
        <span className="mono-xs tabular-nums">{value}</span>
      </div>
      <Meter value={pct} tone={tone ?? (pct > 85 ? "danger" : "primary")} className="mt-1 h-1" />
    </div>
  );
}
