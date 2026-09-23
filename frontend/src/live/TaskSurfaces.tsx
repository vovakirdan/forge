import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { TaskDetailView } from "../contracts/task.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { GitIntegrationRecovery } from "./GitIntegrationRecovery.tsx";

type Kind = "git-source-policy" | "file-inputs" | "file-snapshots" | "reviews" | "integrations";
const sections: { kind: Kind; title: string }[] = [
  { kind: "git-source-policy", title: "Git source policy" },
  { kind: "file-snapshots", title: "File snapshots" },
  { kind: "file-inputs", title: "Attached file inputs" },
  { kind: "reviews", title: "Candidate reviews" },
  { kind: "integrations", title: "Git integrations" },
];

export function TaskSurfaces({ scope, task }: { scope: ProjectReadScope; task: TaskDetailView }) {
  return (
    <section
      aria-label="Task work surface and evidence"
      className="space-y-3 border-t border-border pt-4"
    >
      <h3 className="font-medium">Work surface and evidence</h3>
      <p className="text-xs text-muted-foreground">
        Core work surface: {task.work_surface_kind === "git" ? "Git binding" : "None"}. Git views
        apply only when Core has a Git binding. A review is bound to one candidate; an integration
        receipt is separate from publication. File contents are unavailable here.
      </p>
      {sections.map(({ kind, title }) => (
        <SurfaceSection key={kind} scope={scope} task={task} kind={kind} title={title} />
      ))}
    </section>
  );
}

function SurfaceSection({
  scope,
  task,
  kind,
  title,
}: {
  scope: ProjectReadScope;
  task: TaskDetailView;
  kind: Kind;
  title: string;
}) {
  const { api, session, generation, projectId } = scope;
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const [after, setAfter] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const key = useMemo(
    () => readKeys.taskSurface(generation, projectId, task.id, kind, after),
    [generation, projectId, task.id, kind, after],
  );
  const query = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.taskSurface(projectId, task.id, kind, after, token, signal),
      ),
    enabled: open,
    retry: false,
  });
  useReadLifetime(key);
  function navigate(next: string | null, history: (string | null)[]) {
    prepareReadChange(client, key);
    setAfter(next);
    setPrevious(history);
  }
  const result = query.data;
  const next = result && "next_cursor" in result.value ? result.value.next_cursor : null;
  return (
    <section aria-label={title} className="rounded border border-border p-3 space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="font-medium">{title}</h4>
        <Button variant="outline" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "Hide" : "Show"}
        </Button>
      </div>
      {open && (
        <>
          {query.isPending && <p role="status">Loading {title.toLowerCase()}…</p>}
          {query.isError && (
            <p role="alert">
              {query.error instanceof LiveApiError &&
              query.error.kind === "not_found" &&
              kind === "git-source-policy"
                ? "No Git source policy is available for this Task. Its work-surface type is not exposed by Task detail."
                : `${result ? "Showing stale data. " : ""}${describeApiError(query.error, "Task")}`}
            </p>
          )}
          {result && <SurfaceItems result={result} scope={scope} taskId={task.id} cursor={after} />}
          {result && "next_cursor" in result.value && (
            <nav
              aria-label={`${title} pagination`}
              className="flex items-center justify-between gap-2"
            >
              <Button
                variant="outline"
                disabled={query.isFetching || previous.length === 0}
                onClick={() => navigate(previous.at(-1) ?? null, previous.slice(0, -1))}
              >
                Previous
              </Button>
              <span className="text-xs">Page {previous.length + 1}</span>
              <Button
                variant="outline"
                disabled={query.isFetching || !next}
                onClick={() => {
                  if (next) navigate(next, [...previous, after]);
                }}
              >
                Next
              </Button>
            </nav>
          )}
        </>
      )}
    </section>
  );
}

type SurfaceResult = Awaited<ReturnType<ProjectReadScope["api"]["taskSurface"]>>;
function SurfaceItems({
  result,
  scope,
  taskId,
  cursor,
}: {
  result: SurfaceResult;
  scope: ProjectReadScope;
  taskId: string;
  cursor: string | null;
}) {
  switch (result.kind) {
    case "git-source-policy":
      return (
        <p className="text-sm [overflow-wrap:anywhere]">
          Revision {result.value.revision} ·{" "}
          {result.value.policy.mode === "pinned_commit"
            ? `Pinned commit ${result.value.policy.commit}`
            : "Latest target at future Run preparation"}
        </p>
      );
    case "file-snapshots":
      return (
        <>
          {result.value.items.length === 0 && <p>No snapshots on this page.</p>}
          <ul className="space-y-2">
            {result.value.items.map((snapshot) => (
              <li key={snapshot.id} className="text-sm [overflow-wrap:anywhere]">
                <strong>{snapshot.title}</strong> · {snapshot.state} · {snapshot.created_at}
                {snapshot.state === "sealed" && <> · Artifact {snapshot.artifact_id}</>}
                {snapshot.manifest && <FileList files={snapshot.manifest.files} />}
                {snapshot.error_code && <span> · Error code {snapshot.error_code}</span>}
              </li>
            ))}
          </ul>
        </>
      );
    case "file-inputs":
      return (
        <>
          {result.value.items.length === 0 && <p>No file inputs attached.</p>}
          <ul className="space-y-2">
            {result.value.items.map((input) => (
              <li key={input.artifact_id} className="text-sm [overflow-wrap:anywhere]">
                Artifact {input.artifact_id} · Source Task {input.manifest.source_task_id}
                <FileList files={input.manifest.files} />
              </li>
            ))}
          </ul>
        </>
      );
    case "reviews":
      return (
        <>
          {result.value.items.length === 0 && <p>No reviews on this page.</p>}
          <ul className="space-y-2">
            {result.value.items.map((review) => (
              <li key={review.id} className="text-sm [overflow-wrap:anywhere]">
                Review {review.id} · {review.verdict} · {review.recorded_at}
                <br />
                Stage {review.stage_id}, visit {review.stage_visit} · Proposal{" "}
                {review.candidate_proposal_id} · Candidate commit {review.candidate.commit}
                <br />
                Subject artifacts: {review.subject_artifact_ids.join(", ") || "none"} · Verdict
                artifacts: {review.verdict_artifact_ids.join(", ") || "none"}
              </li>
            ))}
          </ul>
        </>
      );
    case "integrations":
      return (
        <>
          {result.value.items.length === 0 && <p>No integrations on this page.</p>}
          <ul className="space-y-2">
            {result.value.items.map((integration) => (
              <li key={integration.id} className="text-sm [overflow-wrap:anywhere]">
                Integration {integration.id} · {integration.state} · Result{" "}
                {integration.result_code ?? "pending"}
                <br />
                Stage {integration.stage_id}, visit {integration.stage_visit} · Proposal{" "}
                {integration.candidate_proposal_id} · Candidate commit{" "}
                {integration.candidate.commit}
                {integration.prepared_merge && (
                  <span>
                    <br />
                    Prepared merge {integration.prepared_merge.merge_commit}
                  </span>
                )}
                {integration.state === "held" && (
                  <GitIntegrationRecovery
                    scope={scope}
                    taskId={taskId}
                    cursor={cursor}
                    integration={integration}
                  />
                )}
              </li>
            ))}
          </ul>
        </>
      );
  }
}
function FileList({
  files,
}: {
  files: { path: string; size_bytes: number; executable: boolean; sha256: string }[];
}) {
  return (
    <ul className="ml-4 list-disc">
      {files.map((file) => (
        <li key={`${file.path}:${file.sha256}`}>
          {file.path} · {file.size_bytes} bytes · SHA-256 {file.sha256}
          {file.executable ? " · executable" : ""}
        </li>
      ))}
    </ul>
  );
}
