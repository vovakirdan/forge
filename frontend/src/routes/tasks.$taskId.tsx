import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { TaskService } from "@/services";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/common/Bits";
import { TaskActions, TaskDetailBody, TaskSummaryBar } from "@/components/task/TaskDetail";
import { Button } from "@/components/ui/button";
import { ArrowLeft, SearchX } from "lucide-react";

export const Route = createFileRoute("/tasks/$taskId")({
  head: ({ params }) => ({
    meta: [
      { title: `${params.taskId} — Task detail — Forge` },
      {
        name: "description",
        content:
          "Full work item detail: ancestry, definition of done, pipeline timeline, findings, artifacts and runs.",
      },
      { property: "og:title", content: `${params.taskId} — Task detail — Forge` },
      {
        property: "og:description",
        content: "One task, one lifecycle: implementation, verification, review, integration.",
      },
    ],
  }),
  component: TaskPage,
});

function TaskPage() {
  const { taskId } = Route.useParams();
  const { data: task, isLoading } = useQuery({
    queryKey: ["task", taskId],
    queryFn: () => TaskService.get(taskId),
  });

  if (isLoading) {
    return (
      <PageBody className="space-y-3">
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-64 w-full" />
      </PageBody>
    );
  }

  if (!task) {
    return (
      <EmptyState
        icon={<SearchX className="size-5" />}
        title="Task not found"
        hint={`No work item with id ${taskId} exists in this project.`}
        action={
          <Button asChild size="sm" variant="outline" className="mt-2 h-7 text-[12px]">
            <Link to="/board">Back to board</Link>
          </Button>
        }
      />
    );
  }

  return (
    <Page>
      <PageHeader
        title={`${task.id} — ${task.title}`}
        subtitle={`${task.area} · workspace ${task.workspace} · attempt ${task.attempts}`}
        actions={
          <>
            <Button asChild size="sm" variant="ghost" className="h-7 text-[12px]">
              <Link to="/board">
                <ArrowLeft className="size-3.5" /> Board
              </Link>
            </Button>
            <TaskActions task={task} />
          </>
        }
        tabs={<TaskSummaryBar task={task} />}
      />
      <PageBody>
        <TaskDetailBody task={task} />
      </PageBody>
    </Page>
  );
}
