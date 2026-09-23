import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { TaskDetailView } from "../contracts/task.ts";
import { presentTaskDetail } from "../presentation/task.ts";
import type { PriorityCatalog } from "../presentation/priority.ts";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { Field } from "./Field.tsx";
import { TaskFacts } from "./TaskFacts.tsx";
import type { ProjectReadScope } from "./read-scope.ts";
import { TaskDraftEditor } from "./TaskDraftEditor.tsx";
import { TaskPriorityEditor } from "./TaskPriorityEditor.tsx";
import { canChangePriority } from "./priority-attempt.ts";
import { TaskApprovalPanel } from "./TaskApprovalPanel.tsx";
import { TaskCancellationPanel } from "./TaskCancellationPanel.tsx";
import { TaskCancellationFacts } from "./TaskCancellationFacts.tsx";
import { canCancelTask } from "./cancellation-attempt.ts";
import { TaskDependencies } from "./TaskDependencies.tsx";

export function TaskDetailPanel(
  scope: ProjectReadScope & {
    taskId: string;
    onClose: () => void;
    onOpenRelated: (taskId: string) => void;
    onBack: (() => void) | undefined;
    priorities: PriorityCatalog;
  },
) {
  const { api, session, generation, projectId, taskId, onClose } = scope;
  const [editing, setEditing] = useState<"draft" | "priority" | "approval" | "cancellation" | null>(
    null,
  );
  const title = useRef<HTMLHeadingElement>(null);
  const key = useMemo(
    () => readKeys.task(generation, projectId, taskId),
    [generation, projectId, taskId],
  );
  const detail = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.task(projectId, taskId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  useEffect(() => {
    title.current?.focus();
  }, []);
  const task = detail.data;
  return (
    <section
      id="task-detail"
      aria-label="Task detail"
      className="space-y-5 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2
          id="task-detail-title"
          ref={title}
          tabIndex={-1}
          className="rounded text-lg font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Task detail
        </h2>
        <div className="flex flex-wrap gap-2">
          {scope.onBack && (
            <Button variant="outline" onClick={scope.onBack}>
              Back to previous task
            </Button>
          )}
          {!editing &&
            detail.isSuccess &&
            !detail.isFetching &&
            detail.data.lifecycle === "draft" && (
              <>
                <Button onClick={() => setEditing("draft")}>Edit draft</Button>
                <Button onClick={() => setEditing("approval")}>Approve draft</Button>
              </>
            )}
          {!editing &&
            detail.isSuccess &&
            !detail.isFetching &&
            canChangePriority(detail.data.lifecycle) && (
              <Button onClick={() => setEditing("priority")}>Change priority</Button>
            )}
          <Button
            variant="outline"
            disabled={detail.isFetching}
            onClick={() => void detail.refetch()}
          >
            Refresh task
          </Button>
          {!editing &&
            detail.isSuccess &&
            !detail.isFetching &&
            canCancelTask(detail.data.lifecycle) && (
              <Button onClick={() => setEditing("cancellation")}>Cancel task</Button>
            )}
          <Button variant="outline" onClick={onClose}>
            Close task
          </Button>
        </div>
      </div>
      {editing === "draft" && <TaskDraftEditor {...scope} onCancel={() => setEditing(null)} />}
      {editing === "cancellation" && (
        <TaskCancellationPanel {...scope} onCancel={() => setEditing(null)} />
      )}
      {editing === "priority" && (
        <TaskPriorityEditor {...scope} onCancel={() => setEditing(null)} />
      )}
      {editing === "approval" && (
        <TaskApprovalPanel
          {...scope}
          onCancel={() => setEditing(null)}
          onEdit={() => setEditing("draft")}
        />
      )}
      {detail.isPending && <p role="status">Loading Task detail…</p>}
      {detail.isFetching && task && (
        <p role="status">Refreshing Task detail… Previous data remains visible.</p>
      )}
      {detail.isError && (
        <p role="alert">
          {task ? "Showing stale Task detail. " : ""}
          {describeApiError(detail.error, "Task")}
        </p>
      )}
      {task && (
        <>
          <TaskFacts task={task} priorities={scope.priorities} detail />
          {task.cancellation && (
            <TaskCancellationFacts {...scope} cancellation={task.cancellation} />
          )}
          {detail.isSuccess && !detail.isFetching && task.current_stage_id !== null ? (
            <PinnedStage
              key={`${task.pipeline_version_id}:${task.revision}`}
              {...scope}
              task={task}
            />
          ) : (
            <StageName task={task} />
          )}
          <section aria-label="Description" className="space-y-2">
            <h3 className="font-medium">Description</h3>
            <p className="whitespace-pre-wrap text-sm [overflow-wrap:anywhere]">
              {task.description || "No description."}
            </p>
          </section>
          <section aria-label="Definition of done" className="space-y-2">
            <h3 className="font-medium">Definition of done</h3>
            <p className="whitespace-pre-wrap text-sm [overflow-wrap:anywhere]">
              {task.definition_of_done ?? "Not provided."}
            </p>
          </section>
          <TaskProperties task={task} />
          <TaskDependencies
            {...scope}
            taskKey={task.key}
            canEdit={detail.isSuccess && !detail.isFetching}
          />
          <TaskWaits task={task} />
          <TaskArtifacts task={task} />
        </>
      )}
    </section>
  );
}

function StageName({
  task,
  pipeline,
}: {
  task: TaskDetailView;
  pipeline?: Parameters<typeof presentTaskDetail>[1];
}) {
  const { stage } = presentTaskDetail(task, pipeline).presentation;
  const label =
    stage.status === "resolved"
      ? stage.stage.name
      : stage.status === "no_stage"
        ? "No current stage"
        : `Stage unavailable (${stage.reason})`;
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 text-sm">
      <Field label="Stage name">{label}</Field>
    </dl>
  );
}

function PinnedStage({
  api,
  session,
  generation,
  projectId,
  task,
}: ProjectReadScope & { task: TaskDetailView }) {
  const key = useMemo(
    () => readKeys.pipeline(generation, projectId, task.id, task.pipeline_version_id),
    [generation, projectId, task.id, task.pipeline_version_id],
  );
  const pipeline = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.pipeline(projectId, task.pipeline_version_id, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <div className="space-y-2">
      <StageName task={task} pipeline={pipeline.isSuccess ? pipeline.data : undefined} />
      {pipeline.isPending && (
        <p role="status" className="text-sm">
          Loading pinned Pipeline…
        </p>
      )}
      {pipeline.isError && (
        <>
          <p role="alert" className="text-sm">
            Pinned Pipeline unavailable. {describeApiError(pipeline.error, "Pipeline")}
          </p>
          <Button
            variant="secondary"
            disabled={pipeline.isFetching}
            onClick={() => void pipeline.refetch()}
          >
            Retry pipeline
          </Button>
        </>
      )}
    </div>
  );
}

function TaskProperties({ task }: { task: TaskDetailView }) {
  const properties = Object.entries(task.properties);
  return (
    <section aria-label="Properties" className="space-y-3">
      <h3 className="font-medium">Properties</h3>
      {properties.length === 0 ? (
        <p className="text-sm">No properties.</p>
      ) : (
        <dl className="space-y-3 text-sm">
          {properties.map(([name, property]) => (
            <div key={name} className="space-y-1">
              <dt className="font-mono [overflow-wrap:anywhere]">
                {name} ({property.type})
              </dt>
              <dd className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                {JSON.stringify(property.value)}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}

function TaskWaits({ task }: { task: TaskDetailView }) {
  const count = presentTaskDetail(task).presentation.loadedWaitConditionCount;
  return (
    <section aria-label="Wait conditions" className="space-y-3">
      <h3 className="font-medium">Wait conditions ({count} loaded)</h3>
      {count === 0 ? (
        <p className="text-sm">No loaded wait conditions.</p>
      ) : (
        <ul className="space-y-3">
          {task.wait_conditions.map((wait) => (
            <li key={wait.id} className="rounded border border-border p-3">
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
                <Field label="ID">{wait.id}</Field>
                <Field label="Kind">{wait.kind}</Field>
                <Field label="Detail">{wait.detail ?? "Not provided."}</Field>
                <Field label="Source stage ID">{wait.source_stage_id ?? "Not provided."}</Field>
                <Field label="Created by">
                  {wait.created_by.kind}: {wait.created_by.id}
                </Field>
                <Field label="Created at">{wait.created_at}</Field>
              </dl>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function TaskArtifacts({ task }: { task: TaskDetailView }) {
  const count = presentTaskDetail(task).presentation.loadedArtifactCount;
  return (
    <section aria-label="Artifacts" className="space-y-3">
      <h3 className="font-medium">Artifacts ({count} loaded)</h3>
      <p className="text-xs text-muted-foreground">
        Bodies and metadata are not displayed; linked objects are not fetched automatically.
      </p>
      {count === 0 ? (
        <p className="text-sm">No loaded artifacts.</p>
      ) : (
        <ul className="space-y-3">
          {task.artifacts.map((artifact) => (
            <li key={artifact.id} className="rounded border border-border p-3">
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
                <Field label="ID">{artifact.id}</Field>
                <Field label="Kind">{artifact.kind}</Field>
                <Field label="Title">{artifact.title}</Field>
                <Field label="Created at">{artifact.created_at}</Field>
              </dl>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
