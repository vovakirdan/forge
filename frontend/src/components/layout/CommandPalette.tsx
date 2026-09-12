import { useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { AgentService, RunService, TaskService } from "@/services";
import { useProject } from "@/components/layout/project-context";
import {
  Activity,
  Bot,
  BookOpen,
  Columns3,
  Cpu,
  GitBranch,
  LayoutDashboard,
  MessageSquare,
  Play,
  Settings,
  SquareCheck,
  Target,
} from "lucide-react";

const pages = [
  { to: "/", label: "Overview", icon: LayoutDashboard },
  { to: "/board", label: "Board", icon: Columns3 },
  { to: "/goals", label: "Goals", icon: Target },
  { to: "/team", label: "Team", icon: Bot },
  { to: "/pipelines", label: "Pipelines", icon: GitBranch },
  { to: "/chat", label: "Chat with Lead", icon: MessageSquare },
  { to: "/activity", label: "Activity", icon: Activity },
  { to: "/knowledge", label: "Knowledge", icon: BookOpen },
  { to: "/resources", label: "Resources", icon: Cpu },
  { to: "/settings", label: "Settings", icon: Settings },
] as const;

export function CommandPalette() {
  const { commandOpen, setCommandOpen } = useProject();
  const navigate = useNavigate();
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: runs } = useQuery({ queryKey: ["runs"], queryFn: RunService.list });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setCommandOpen(!commandOpen);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [commandOpen, setCommandOpen]);

  const go = (fn: () => void) => {
    setCommandOpen(false);
    fn();
  };

  return (
    <CommandDialog open={commandOpen} onOpenChange={setCommandOpen}>
      <CommandInput placeholder="Jump to a task, agent, run or page…" />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>
        <CommandGroup heading="Navigate">
          {pages.map(({ to, label, icon: Icon }) => (
            <CommandItem
              key={to}
              value={`page ${label}`}
              onSelect={() => go(() => navigate({ to }))}
            >
              <Icon className="size-3.5 text-muted-foreground" />
              {label}
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Tasks">
          {tasks?.map((t) => (
            <CommandItem
              key={t.id}
              value={`${t.id} ${t.title}`}
              onSelect={() =>
                go(() => navigate({ to: "/tasks/$taskId", params: { taskId: t.id } }))
              }
            >
              <SquareCheck className="size-3.5 text-muted-foreground" />
              <span className="mono-xs text-muted-foreground">{t.id}</span>
              <span className="truncate">{t.title}</span>
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Employees">
          {agents?.map((a) => (
            <CommandItem
              key={a.id}
              value={`${a.name} ${a.role}`}
              onSelect={() =>
                go(() => navigate({ to: "/team/$agentId", params: { agentId: a.id } }))
              }
            >
              <Bot className="size-3.5 text-muted-foreground" />
              {a.name}
              <span className="text-muted-foreground">{a.role}</span>
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Runs">
          {runs?.slice(0, 6).map((r) => (
            <CommandItem
              key={r.id}
              value={`${r.id} ${r.reason}`}
              onSelect={() => go(() => navigate({ to: "/runs/$runId", params: { runId: r.id } }))}
            >
              <Play className="size-3.5 text-muted-foreground" />
              <span className="mono-xs text-muted-foreground">{r.id}</span>
              <span className="truncate">{r.reason}</span>
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
