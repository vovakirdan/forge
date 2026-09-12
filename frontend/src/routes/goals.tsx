import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { GoalService, TaskService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import {
  Chip,
  Dot,
  EmptyState,
  Meter,
  Panel,
  StageBadge,
  priorityTone,
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
} from "@/components/ui/alert-dialog";
import { toast } from "sonner";
import { Lightbulb, Layers, Waves } from "lucide-react";

export const Route = createFileRoute("/goals")({
  head: () => ({
    meta: [
      { title: "Goals & Waves — Forge" },
      {
        name: "description",
        content:
          "Goal, epic and wave planning. Only the current wave holds executable tasks; future work stays as ideas.",
      },
      { property: "og:title", content: "Goals & Waves — Forge" },
      { property: "og:description", content: "Rolling-wave planning prevents backlog explosion." },
    ],
  }),
  component: GoalsPage,
});

function GoalsPage() {
  const { data: goals } = useQuery({ queryKey: ["goals"], queryFn: GoalService.list });
  const { data: epics } = useQuery({ queryKey: ["epics"], queryFn: () => GoalService.epics() });
  const { data: tasks } = useQuery({ queryKey: ["tasks"], queryFn: TaskService.list });
  const [selectedGoal, setSelectedGoal] = useState("GOAL-7");
  const [openEpic, setOpenEpic] = useState<string | null>("EPIC-1");
  const [planEpic, setPlanEpic] = useState<string | null>(null);

  if (!goals || !epics || !tasks) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-72 w-full" />
      </PageBody>
    );
  }

  const goal = goals.find((g) => g.id === selectedGoal) ?? goals[0];
  if (!goal) return null;
  const goalEpics = epics.filter((e) => e.goalId === goal.id);
  const waveTasks = tasks.filter(
    (t) =>
      goalEpics.some((e) => e.id === t.epicId) &&
      ["ready", "planned", "implementation", "verification", "review"].includes(t.stage),
  );

  return (
    <Page>
      <PageHeader
        title="Goals"
        subtitle="Goal → Epic → Work item. Planning happens in waves, not all at once."
        actions={
          <Chip tone="info" icon={<Waves className="size-2.5" />}>
            Wave 3 open · {waveTasks.length} executable tasks
          </Chip>
        }
      />
      <PageBody className="space-y-3">
        <div className="grid gap-3 lg:grid-cols-[260px_1fr]">
          <Panel title="Goals" dense>
            <ul className="divide-y divide-border">
              {goals.map((g) => (
                <li key={g.id}>
                  <button
                    onClick={() => setSelectedGoal(g.id)}
                    className={`w-full px-3 py-2 text-left transition-colors hover:bg-accent/50 ${g.id === goal.id ? "bg-accent/60" : ""}`}
                  >
                    <div className="flex items-center gap-1.5">
                      <Dot tone={g.status === "active" ? "success" : "muted"} />
                      <span className="mono-xs text-muted-foreground">{g.id}</span>
                      <Chip tone={g.status === "active" ? "success" : "muted"} className="ml-auto">
                        {g.status}
                      </Chip>
                    </div>
                    <p className="mt-1 text-[12.5px] font-medium leading-snug">{g.title}</p>
                    <div className="mt-1.5 flex items-center gap-1.5">
                      <Meter value={g.progress} className="h-1" />
                      <span className="mono-xs">{g.progress}%</span>
                    </div>
                  </button>
                </li>
              ))}
            </ul>
          </Panel>

          <div className="space-y-3">
            <Panel title="Goal detail">
              <h2 className="text-[13.5px] font-semibold">{goal.title}</h2>
              <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
                {goal.description}
              </p>
              <div className="mt-3 flex items-center gap-2">
                <Meter value={goal.progress} />
                <span className="mono-xs tabular-nums">{goal.progress}%</span>
              </div>
            </Panel>

            {goalEpics.length === 0 ? (
              <Panel>
                <EmptyState
                  icon={<Layers className="size-5" />}
                  title="No epics planned yet"
                  hint="The Lead decomposes a goal into epics before any executable work is created."
                />
              </Panel>
            ) : (
              goalEpics.map((epic) => {
                const open = openEpic === epic.id;
                const epicTasks = tasks.filter((t) => t.epicId === epic.id);
                return (
                  <Panel
                    key={epic.id}
                    title={
                      <button
                        className="flex items-center gap-2"
                        onClick={() => setOpenEpic(open ? null : epic.id)}
                      >
                        <span className="mono-xs">{epic.id}</span>
                        <span className="text-[12px] font-semibold normal-case tracking-normal text-foreground">
                          {epic.title}
                        </span>
                        <Chip tone="muted">{epic.area}</Chip>
                      </button>
                    }
                    action={
                      <div className="flex items-center gap-2">
                        <Meter value={epic.progress} className="w-24" />
                        <span className="mono-xs tabular-nums">{epic.progress}%</span>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 text-[11px]"
                          onClick={() => setPlanEpic(epic.id)}
                        >
                          Plan next wave
                        </Button>
                      </div>
                    }
                  >
                    {open && (
                      <div className="grid gap-4 md:grid-cols-2">
                        <div>
                          <p className="section-label mb-1.5">
                            Current planned wave · {epic.waveTaskIds.length} tasks
                          </p>
                          <ul className="space-y-1">
                            {epicTasks
                              .filter((t) => epic.waveTaskIds.includes(t.id))
                              .map((t) => (
                                <li key={t.id}>
                                  <Link
                                    to="/tasks/$taskId"
                                    params={{ taskId: t.id }}
                                    className="flex items-center gap-2 rounded border border-border bg-surface px-2 py-1.5 hover:border-border-strong"
                                  >
                                    <span className="mono-xs text-muted-foreground">{t.id}</span>
                                    <span className="min-w-0 flex-1 truncate text-[12px]">
                                      {t.title}
                                    </span>
                                    <Chip tone={priorityTone[t.priority]}>{t.priority}</Chip>
                                    <StageBadge stage={t.stage} />
                                  </Link>
                                </li>
                              ))}
                            {epic.waveTaskIds.length === 0 && (
                              <li className="text-[11.5px] text-muted-foreground">
                                No executable tasks in this wave yet.
                              </li>
                            )}
                          </ul>
                        </div>
                        <div>
                          <p className="section-label mb-1.5">
                            Future work — high-level ideas only
                          </p>
                          <ul className="space-y-1">
                            {epic.futureIdeas.map((idea) => (
                              <li
                                key={idea}
                                className="flex items-start gap-2 rounded border border-dashed border-border px-2 py-1.5 text-[12px] text-muted-foreground"
                              >
                                <Lightbulb className="mt-0.5 size-3 shrink-0" />
                                {idea}
                              </li>
                            ))}
                          </ul>
                          <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
                            Ideas are not executable and never appear on the board. They become
                            tasks only when a wave is planned.
                          </p>
                        </div>
                      </div>
                    )}
                  </Panel>
                );
              })
            )}
          </div>
        </div>
      </PageBody>

      <AlertDialog open={!!planEpic} onOpenChange={(o) => !o && setPlanEpic(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="text-[14px]">Plan the next wave?</AlertDialogTitle>
            <AlertDialogDescription className="text-[12.5px]">
              The Lead will convert up to 5 high-level ideas from {planEpic} into executable work
              items with a definition of done. Existing tasks are untouched, and no work starts
              until the scheduler admits a run.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="h-7 text-[12px]">Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="h-7 text-[12px]"
              onClick={() =>
                toast.success("Wave planned", {
                  description: "3 new work items created in Planned.",
                })
              }
            >
              Plan wave
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Page>
  );
}
