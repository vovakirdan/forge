import { useQuery } from "@tanstack/react-query";
import { Search, GitBranch, Command as CommandIcon, Bell, SunMoon } from "lucide-react";
import { ProjectService, RunService } from "@/services";
import { useProject } from "@/components/layout/project-context";
import { Chip, Dot, Hint } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Link } from "@tanstack/react-router";

export function TopBar() {
  const { projectId, setCommandOpen } = useProject();
  const { data: projects } = useQuery({ queryKey: ["projects"], queryFn: ProjectService.list });
  const { data: activeRuns } = useQuery({
    queryKey: ["runs", "active"],
    queryFn: RunService.active,
  });
  const project = projects?.find((p) => p.id === projectId) ?? projects?.[0];

  const toggleTheme = () => document.documentElement.classList.toggle("dark");

  return (
    <header className="flex h-11 shrink-0 items-center gap-3 border-b border-border bg-surface px-3">
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate text-[12.5px] font-semibold">{project?.name}</span>
        <Chip tone="muted" mono icon={<GitBranch className="size-2.5" />}>
          {project?.defaultBranch}
        </Chip>
        <Chip tone={project?.environmentStatus === "healthy" ? "success" : "warning"}>
          <Dot tone={project?.environmentStatus === "healthy" ? "success" : "warning"} />
          {project?.environment} · {project?.environmentStatus}
        </Chip>
      </div>

      <button
        onClick={() => setCommandOpen(true)}
        className="ml-auto flex h-7 w-72 items-center gap-2 rounded-md border border-border bg-background px-2 text-[12px] text-muted-foreground transition-colors hover:border-border-strong"
      >
        <Search className="size-3.5" />
        <span className="flex-1 text-left">Search tasks, agents, runs…</span>
        <kbd className="mono-xs flex items-center gap-0.5 rounded border border-border px-1 py-px">
          <CommandIcon className="size-2.5" />K
        </kbd>
      </button>

      <Chip tone="running">
        <Dot tone="running" pulse />
        {activeRuns?.length ?? 0} runs
      </Chip>

      <Hint label="Notifications">
        <Button asChild variant="ghost" size="icon" className="size-7">
          <Link to="/activity">
            <Bell className="size-3.5" />
          </Link>
        </Button>
      </Hint>
      <Hint label="Toggle theme">
        <Button variant="ghost" size="icon" className="size-7" onClick={toggleTheme}>
          <SunMoon className="size-3.5" />
        </Button>
      </Hint>
    </header>
  );
}
