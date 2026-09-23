import type { MouseEvent } from "react";
import type { TaskSummaryView } from "../contracts/task.ts";
import { taskBoardLanes } from "../presentation/task-board.ts";
import { presentTaskSummary } from "../presentation/task.ts";
import { priorityLabel, type PriorityCatalog } from "../presentation/priority.ts";

export function TaskBoard({
  tasks,
  priorities,
  onOpenTask,
}: {
  tasks: readonly TaskSummaryView[];
  priorities: PriorityCatalog;
  onOpenTask: (task: TaskSummaryView, opener: HTMLButtonElement) => void;
}) {
  return (
    <div aria-label="Task board" className="overflow-x-auto pb-2">
      <div className="flex min-w-full items-start gap-3">
        {taskBoardLanes(tasks).map((lane) => (
          <section
            key={lane.key}
            aria-label={
              lane.stageId === null
                ? "No current stage"
                : `Stage ${lane.stageId} in Pipeline version ${lane.pipelineVersionId}`
            }
            className="w-72 min-w-72 space-y-3 rounded-lg border border-border bg-muted/30 p-3"
          >
            <h3 className="font-semibold [overflow-wrap:anywhere]">
              {lane.stageId === null ? "No current stage" : `Stage ID: ${lane.stageId}`}
            </h3>
            {lane.pipelineVersionId && (
              <p className="text-xs text-muted-foreground [overflow-wrap:anywhere]">
                Pinned Pipeline version: {lane.pipelineVersionId}
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              {lane.tasks.length} {lane.tasks.length === 1 ? "Task" : "Tasks"} on this page
            </p>
            <ul className="space-y-3">
              {lane.tasks.map((task) => {
                const { presentation } = presentTaskSummary(task);
                return (
                  <li
                    key={task.id}
                    className="space-y-2 rounded-md border border-border bg-card p-3"
                  >
                    <button
                      type="button"
                      aria-label={`Open ${task.key}`}
                      aria-controls="task-detail"
                      onClick={(event: MouseEvent<HTMLButtonElement>) =>
                        onOpenTask(task, event.currentTarget)
                      }
                      className="w-full cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
                    >
                      {task.key} — {task.title}
                    </button>
                    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 text-xs [overflow-wrap:anywhere]">
                      <dt>Lifecycle</dt>
                      <dd>{presentation.lifecycleLabel}</dd>
                      <dt>Kind</dt>
                      <dd>{presentation.kindLabel}</dd>
                      <dt>Priority</dt>
                      <dd>{priorityLabel(task.priority, priorities)}</dd>
                    </dl>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}
