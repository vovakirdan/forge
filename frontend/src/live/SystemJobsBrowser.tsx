import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Field } from "./Field.tsx";
import { describeApiError } from "./api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { SystemJobCommandButton } from "./SystemJobCommandButton.tsx";
import {
  SystemJobBindingInputSchema,
  SystemJobPolicyInputSchema,
} from "../contracts/system-job-command.ts";

const defaultPolicy = {
  enabled: false,
  max_concurrent: 1,
  max_attempts_per_job: 2,
  max_attempts_per_day: 20,
  max_input_bytes: 65_536,
  max_result_bytes: 16_384,
  wall_seconds: 300,
  coalesce_seconds: 5,
};

function SystemJobAttempts({ scope, jobId }: { scope: ProjectReadScope; jobId: string }) {
  const { api, session, generation, projectId } = scope;
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const key = useMemo(
    () => readKeys.systemJobAttempts(generation, projectId, jobId, cursor),
    [generation, projectId, jobId, cursor],
  );
  const page = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.systemJobAttempts(projectId, jobId, cursor, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section
      aria-label={`Attempts for System Job ${jobId}`}
      className="mt-3 border-t border-border pt-3"
    >
      <div className="flex items-center justify-between gap-2">
        <h4 className="text-sm font-medium">Attempts for this job</h4>
        <Button variant="outline" disabled={page.isFetching} onClick={() => void page.refetch()}>
          Refresh attempts
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Each attempt pins one Run. An accepted result can later be superseded; derived entries are
        shown only after completed materialization. Open Runs or Knowledge by the IDs below.
      </p>
      {page.isPending && <p role="status">Loading attempts…</p>}
      {page.isError && <p role="alert">{describeApiError(page.error, "Project")}</p>}
      {page.data && (
        <>
          {page.data.items.length === 0 ? (
            <p className="text-sm">No attempts on this page.</p>
          ) : (
            <ol className="space-y-2 text-sm">
              {page.data.items.map((attempt) => (
                <li
                  key={attempt.id}
                  className="rounded border border-border p-2 [overflow-wrap:anywhere]"
                >
                  <p className="font-medium">Attempt {attempt.id}</p>
                  <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3">
                    <Field label="Generation">{attempt.generation}</Field>
                    <Field label="State">{attempt.state}</Field>
                    <Field label="Run ID">{attempt.run_id}</Field>
                    <Field label="Result accepted">{attempt.result_accepted ? "Yes" : "No"}</Field>
                    <Field label="Result message ID">{attempt.result_message_id ?? "None"}</Field>
                    <Field label="Created">{attempt.created_at}</Field>
                    <Field label="Completed">{attempt.completed_at ?? "Not completed"}</Field>
                  </dl>
                  <p className="mt-1 text-xs">
                    Materialized derived entries: {attempt.materialized_entries.length}
                  </p>
                  {attempt.materialized_entries.length > 0 && (
                    <ul className="ml-4 list-disc text-xs">
                      {attempt.materialized_entries.map((entry) => (
                        <li key={`${entry.id}:${entry.revision}`}>
                          {entry.id} · revision {entry.revision}
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ol>
          )}
          <div className="mt-2 flex gap-2">
            <Button
              variant="outline"
              disabled={page.isFetching || previous.length === 0}
              onClick={() => {
                setCursor(previous[previous.length - 1] ?? null);
                setPrevious(previous.slice(0, -1));
              }}
            >
              Previous attempts
            </Button>
            <Button
              variant="outline"
              disabled={page.isFetching || !page.data.next_cursor}
              onClick={() => {
                if (!page.data.next_cursor) return;
                setPrevious([...previous, cursor]);
                setCursor(page.data.next_cursor);
              }}
            >
              Next attempts
            </Button>
          </div>
        </>
      )}
    </section>
  );
}

export function SystemJobsBrowser(scope: ProjectReadScope) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const [taskId, setTaskId] = useState("");
  const [policyText, setPolicyText] = useState(JSON.stringify(defaultPolicy, null, 2));
  const [bindingText, setBindingText] = useState("");
  const [selectedJob, setSelectedJob] = useState<string | null>(null);
  useEffect(() => {
    if (!taskId && policyText === JSON.stringify(defaultPolicy, null, 2) && !bindingText) return;
    return leaveGuard.register(
      () => true,
      "Leave System Job settings? Unsaved policy, binding, or Task ID will be lost.",
    );
  }, [leaveGuard, taskId, policyText, bindingText]);
  const key = useMemo(() => readKeys.systemJobs(generation, projectId), [generation, projectId]);
  const status = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.systemJobs(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  const data = status.data;
  return (
    <section
      aria-label="System Jobs"
      className="space-y-5 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">System Jobs</h2>
        <Button
          variant="outline"
          disabled={status.isFetching}
          onClick={() => void status.refetch()}
        >
          Refresh status
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Core status and audited controls. A pending or running job is not proof of a started
        provider Run. Open a job to read its bounded attempt history.
      </p>
      <section aria-label="System Job commands" className="space-y-3 border-t border-border pt-4">
        <h3 className="font-medium">Manage System Jobs</h3>
        <p className="text-xs text-muted-foreground">
          Configuration requires a complete RuntimeBinding using an enrolled Project credential and
          surface mode none. Disabling requests stops for active System Job Runs.
        </p>
        <label className="block text-sm">
          Policy JSON
          <textarea
            value={policyText}
            onChange={(event) => setPolicyText(event.target.value)}
            rows={8}
            className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
          />
        </label>
        <label className="block text-sm">
          RuntimeBinding JSON
          <textarea
            value={bindingText}
            onChange={(event) => setBindingText(event.target.value)}
            rows={10}
            className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
          />
        </label>
        <SystemJobCommandButton
          {...scope}
          action="configure_system_jobs"
          label="Save System Job configuration"
          payload={() => ({
            expected_settings_revision: data?.settings_revision ?? 0,
            policy: SystemJobPolicyInputSchema.parse(JSON.parse(policyText) as unknown),
            binding: SystemJobBindingInputSchema.parse(JSON.parse(bindingText) as unknown),
          })}
          onApplied={() => void status.refetch()}
        />
        <label className="block text-sm">
          Task ID for summary
          <input
            value={taskId}
            onChange={(event) => setTaskId(event.target.value)}
            className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-sm"
          />
        </label>
        <SystemJobCommandButton
          {...scope}
          action="request_task_summary"
          label="Request Task summary"
          disabled={!data?.policy}
          payload={() => ({ task_id: taskId })}
          onApplied={() => void status.refetch()}
        />
      </section>
      {status.isPending && <p role="status">Loading System Jobs…</p>}
      {status.isFetching && data && (
        <p role="status">Refreshing status… Previous data remains visible.</p>
      )}
      {status.isError && (
        <p role="alert">
          {data ? "Showing stale System Job status. " : ""}
          {describeApiError(status.error, "Project")}
        </p>
      )}
      {data && (
        <>
          <section aria-label="System Job policy" className="space-y-2">
            <h3 className="font-medium">Policy</h3>
            {data.policy ? (
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
                <Field label="Enabled">{data.policy.enabled ? "Yes" : "No"}</Field>
                <Field label="Settings revision">{data.settings_revision ?? "Unknown"}</Field>
                <Field label="Concurrent limit">{data.policy.max_concurrent}</Field>
                <Field label="Attempts per job generation">
                  {data.policy.max_attempts_per_job}
                </Field>
                <Field label="Attempts per day">{data.policy.max_attempts_per_day}</Field>
                <Field label="Input byte limit">{data.policy.max_input_bytes}</Field>
                <Field label="Result byte limit">{data.policy.max_result_bytes}</Field>
                <Field label="Wall seconds">{data.policy.wall_seconds}</Field>
              </dl>
            ) : (
              <p>No System Job policy configured.</p>
            )}
          </section>
          <section aria-label="System Job usage" className="space-y-2 border-t border-border pt-4">
            <h3 className="font-medium">Attempt records</h3>
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
              <Field label="Created in last 24 hours">{data.usage.attempts_last_24h}</Field>
              <Field label="Marked running">{data.usage.running}</Field>
              <Field label="Provider cost and usage">Unknown; see per-Run accounting</Field>
            </dl>
          </section>
          <section
            aria-label="System Job records"
            className="space-y-3 border-t border-border pt-4"
          >
            <h3 className="font-medium">Jobs in this response ({data.jobs.length})</h3>
            <p className="text-xs text-muted-foreground">
              Core returns at most 256 jobs here; this is not a complete history or an attempt list.
            </p>
            {data.jobs.length === 0 ? (
              <p>No System Jobs in this response.</p>
            ) : (
              <ul className="space-y-3">
                {data.jobs.map((job) => (
                  <li key={job.id} className="rounded border border-border p-3">
                    <p className="font-medium [overflow-wrap:anywhere]">
                      {job.kind} · {job.id}
                    </p>
                    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 text-sm [overflow-wrap:anywhere]">
                      <Field label="State">{job.state}</Field>
                      <Field label="Generation">{job.generation}</Field>
                      <Field label="Covered Event sequence">{job.covered_sequence}</Field>
                      <Field label="Source Task">{job.source_task_id ?? "None"}</Field>
                      <Field label="Target Employee">{job.target_employee_id ?? "None"}</Field>
                      <Field label="Reason code">{job.reason_code ?? "None recorded"}</Field>
                    </dl>
                    {(job.state === "held" || job.state === "cancelled") && (
                      <SystemJobCommandButton
                        {...scope}
                        action="retry_system_job"
                        identity={job.id}
                        label={`Retry job ${job.id}`}
                        payload={() => ({ job_id: job.id })}
                        onApplied={() => void status.refetch()}
                      />
                    )}
                    <Button
                      variant="outline"
                      aria-expanded={selectedJob === job.id}
                      onClick={() => setSelectedJob(selectedJob === job.id ? null : job.id)}
                    >
                      {selectedJob === job.id ? "Close attempts" : "Show attempts"}
                    </Button>
                    {selectedJob === job.id && (
                      <SystemJobAttempts key={job.id} scope={scope} jobId={job.id} />
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>
          <p className="text-xs text-muted-foreground">
            Onboarding records in this response: {data.onboarding.length}. Read each Employee’s
            current onboarding state in their profile. Legacy bypass and explicit skip are distinct
            from a completed familiarity receipt.
          </p>
        </>
      )}
    </section>
  );
}
