import type { ReactNode } from "react";
import type { TaskSummaryView } from "../contracts/task.ts";
import { presentTaskSummary } from "../presentation/task.ts";

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 whitespace-pre-wrap [overflow-wrap:anywhere]">{children}</dd>
    </>
  );
}

export function TaskFacts({ task, detail = false }: { task: TaskSummaryView; detail?: boolean }) {
  const { presentation } = presentTaskSummary(task);
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
      {detail && (
        <>
          <Field label="ID">{task.id}</Field>
          <Field label="Key">{task.key}</Field>
          <Field label="Revision">{task.revision}</Field>
          <Field label="Title">{task.title}</Field>
        </>
      )}
      <Field label="Kind">
        {presentation.kindLabel} ({task.kind})
      </Field>
      <Field label="Lifecycle">
        {presentation.lifecycleLabel} ({task.lifecycle})
      </Field>
      <Field label="Stage ID">{task.current_stage_id ?? "No current stage"}</Field>
      <Field label="Priority ID">{task.priority}</Field>
      {detail && <Field label="Pipeline version ID">{task.pipeline_version_id}</Field>}
      <Field label="Updated at">{task.updated_at}</Field>
    </dl>
  );
}
