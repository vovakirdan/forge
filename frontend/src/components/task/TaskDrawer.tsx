import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { TaskService } from "@/services";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { TaskActions, TaskDetailBody, TaskSummaryBar } from "@/components/task/TaskDetail";
import { ExternalLink } from "lucide-react";
import { useRef } from "react";

export function TaskDrawer({ taskId, onClose }: { taskId: string | null; onClose: () => void }) {
  const returnFocus = useRef<HTMLElement | null>(null);
  const navigating = useRef(false);
  const { data: task, isLoading } = useQuery({
    queryKey: ["task", taskId],
    queryFn: () => TaskService.get(taskId!),
    enabled: !!taskId,
  });

  return (
    <Sheet open={!!taskId} onOpenChange={(o) => !o && onClose()}>
      <SheetContent
        side="right"
        className="w-full gap-0 p-0 sm:max-w-[820px]"
        onOpenAutoFocus={() => {
          returnFocus.current =
            document.activeElement instanceof HTMLElement ? document.activeElement : null;
          navigating.current = false;
        }}
        onCloseAutoFocus={(event) => {
          // This controlled Sheet has no Radix Trigger to restore automatically.
          event.preventDefault();
          if (!navigating.current && returnFocus.current?.isConnected) returnFocus.current.focus();
          returnFocus.current = null;
        }}
      >
        <SheetHeader className="shrink-0 space-y-2 border-b border-border bg-surface p-3">
          <SheetDescription className="sr-only">
            Task details and available actions. Open the full page for a dedicated view.
          </SheetDescription>
          <div className="flex items-start gap-2">
            <SheetTitle className="text-[14px] font-semibold">
              {task ? `${task.id} — ${task.title}` : "Loading task"}
            </SheetTitle>
            <div className="ml-auto flex shrink-0 items-center gap-2">
              {task && (
                <>
                  <Button asChild size="sm" variant="ghost" className="h-7 text-[12px]">
                    <Link
                      to="/tasks/$taskId"
                      params={{ taskId: task.id }}
                      onClick={() => {
                        navigating.current = true;
                        onClose();
                      }}
                    >
                      <ExternalLink className="size-3.5" /> Full page
                    </Link>
                  </Button>
                  <TaskActions task={task} />
                </>
              )}
            </div>
          </div>
          {task && <TaskSummaryBar task={task} />}
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-auto p-3">
          {isLoading || !task ? (
            <div className="space-y-3">
              <Skeleton className="h-24 w-full" />
              <Skeleton className="h-40 w-full" />
              <Skeleton className="h-56 w-full" />
            </div>
          ) : (
            <TaskDetailBody task={task} />
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
