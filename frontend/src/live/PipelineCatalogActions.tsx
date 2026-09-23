import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { UuidV7Schema } from "../contracts/common.ts";
import {
  definitionFromVersion,
  pipelineManagementAttempt,
  PipelineDefinitionInputSchema,
  type PipelineManagementAction,
  type PipelineManagementAttempt,
} from "../contracts/pipeline-management.ts";
import type { PipelineCatalogItem } from "../contracts/pipeline.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendPipelineManagementCommand } from "./pipeline-management-api.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Props = ProjectReadScope & {
  item: PipelineCatalogItem;
  cursor: string | null;
  onClose: () => void;
  onApplied: () => void;
};
type Status =
  | { kind: "editing" }
  | { kind: "sending" | "unknown"; attempt: PipelineManagementAttempt }
  | { kind: "refused"; error: unknown }
  | { kind: "accepted"; attempt: PipelineManagementAttempt; receiptId: string; readError: boolean };

export function PipelineCatalogActions({
  item,
  cursor,
  onClose,
  onApplied,
  api,
  session,
  generation,
  projectId,
  leaveGuard,
}: Props) {
  const queries = useQueryClient();
  const [definitionText, setDefinitionText] = useState("");
  const [sourceId, setSourceId] = useState(item.latest_version_id ?? "");
  const [defaultId, setDefaultId] = useState(item.default_version_id ?? "");
  const [makeDefault, setMakeDefault] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "editing" });
  const [confirm, setConfirm] = useState<PipelineManagementAction | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [acknowledgedRevision, setAcknowledgedRevision] = useState(item.revision);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const confirmRef = useRef<HTMLHeadingElement>(null);
  const baselineKey = useMemo(
    () => ["live", generation, projectId, "pipeline-management-baseline", item.id] as const,
    [generation, projectId, item.id],
  );
  const baseline = useQuery({
    queryKey: baselineKey,
    queryFn: async ({ signal }) => {
      const [project, catalog] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, signal)),
        session.request(generation, (token) =>
          api.pipelineCatalog(projectId, cursor, token, signal),
        ),
      ]);
      const current = catalog.items.find(
        (value) => value.id.toLowerCase() === item.id.toLowerCase(),
      );
      if (!current) throw new LiveApiError("invalid_response");
      return { project, current, catalog };
    },
    retry: false,
  });
  useReadLifetime(baselineKey);
  const sourceKey = useMemo(
    () => ["live", generation, projectId, "pipeline-publication-source", sourceId] as const,
    [generation, projectId, sourceId],
  );
  const source = useQuery({
    queryKey: sourceKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.pipeline(projectId, sourceId, token, signal)),
    enabled: UuidV7Schema.safeParse(sourceId).success,
    retry: false,
  });
  useReadLifetime(sourceKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    titleRef.current?.focus();
    return () => next.abort();
  }, []);
  useEffect(() => {
    if (confirm) confirmRef.current?.focus();
  }, [confirm]);
  const dirty =
    definitionText !== "" ||
    sourceId !== (item.latest_version_id ?? "") ||
    defaultId !== (item.default_version_id ?? "") ||
    makeDefault ||
    status.kind === "unknown" ||
    status.kind === "sending";
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Pipeline management? Unsaved graph and the retry key will be lost. An in-flight command may still have been applied by Core.",
    );
    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [dirty, leaveGuard]);
  function current(next: AbortController) {
    return !next.signal.aborted && session.getSnapshot().generation === generation;
  }
  async function readAccepted(attempt: PipelineManagementAttempt, receiptId: string) {
    const next = controller.current;
    if (!next) return;
    setStatus({ kind: "accepted", attempt, receiptId, readError: false });
    try {
      const [project, catalog] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, next.signal)),
        session.request(generation, (token) =>
          api.pipelineCatalog(projectId, cursor, token, next.signal),
        ),
      ]);
      const row = catalog.items.find((value) => value.id.toLowerCase() === item.id.toLowerCase());
      if (
        !row ||
        row.revision <= (baseline.data?.current.revision ?? item.revision) ||
        project.revision < attempt.expectedProjectRevision + 1
      )
        throw new Error("catalog readback did not confirm command");
      if (attempt.action === "publish_pipeline_version") {
        const published = await session.request(generation, (token) =>
          api.pipeline(projectId, receiptId, token, next.signal),
        );
        if (
          published.pipeline_id.toLowerCase() !== item.id.toLowerCase() ||
          row.latest_version_id?.toLowerCase() !== receiptId.toLowerCase()
        )
          throw new Error("published graph readback did not match receipt");
      }
      if (attempt.action === "delete_pipeline" && !row.deleted_at)
        throw new Error("deletion readback absent");
      if (attempt.action === "set_pipeline_default_version") {
        const intended = JSON.parse(attempt.body) as { payload: { pipeline_version_id: string } };
        if (
          row.default_version_id?.toLowerCase() !==
          intended.payload.pipeline_version_id.toLowerCase()
        )
          throw new Error("default readback absent");
      }
      if (!current(next)) return;
      await Promise.all([
        queries.cancelQueries({ queryKey: ["project", generation, projectId], exact: true }),
        queries.cancelQueries({
          queryKey: ["live", generation, projectId, "pipeline-catalog", cursor],
          exact: true,
        }),
      ]);
      queries.setQueryData(["project", generation, projectId], project);
      queries.setQueryData(["live", generation, projectId, "pipeline-catalog", cursor], catalog);
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "pipeline-versions"],
      });
      onApplied();
    } catch {
      if (current(next)) setStatus({ kind: "accepted", attempt, receiptId, readError: true });
    }
  }
  async function send(attempt: PipelineManagementAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setStatus({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendPipelineManagementCommand(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt, receipt.resource.id);
    } catch (error) {
      if (current(next))
        setStatus(
          error instanceof LiveCommandError && error.kind !== "outcome_unknown"
            ? { kind: "refused", error }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  async function prepare(action: PipelineManagementAction) {
    if (!baseline.data || baseline.isFetching || status.kind !== "editing") return;
    const row = baseline.data.current;
    if (row.revision !== acknowledgedRevision || row.deleted_at) {
      setFieldError("Pipeline catalog changed. Refresh the baseline before sending.");
      return;
    }
    let payload: object = { pipeline_id: item.id, expected_pipeline_revision: row.revision };
    if (action === "publish_pipeline_version") {
      try {
        const definition = PipelineDefinitionInputSchema.parse(
          JSON.parse(definitionText) as unknown,
        );
        payload = { ...payload, definition, make_default: makeDefault };
      } catch {
        setFieldError("The graph JSON does not match a complete Pipeline definition.");
        return;
      }
    }
    if (action === "set_pipeline_default_version") {
      const parsed = UuidV7Schema.safeParse(defaultId);
      if (!parsed.success) {
        setFieldError("Enter a Pipeline version ID.");
        return;
      }
      try {
        const version = await session.request(generation, (token) =>
          api.pipeline(
            projectId,
            parsed.data,
            token,
            controller.current?.signal ?? new AbortController().signal,
          ),
        );
        if (version.pipeline_id.toLowerCase() !== item.id.toLowerCase())
          throw new Error("foreign Pipeline version");
      } catch {
        setFieldError("That version is unavailable in this Pipeline. Check the ID and refresh.");
        return;
      }
      payload = { ...payload, pipeline_version_id: parsed.data };
    }
    try {
      const attempt = pipelineManagementAttempt(action, {
        project_id: projectId,
        expected_revision: baseline.data.project.revision,
        payload,
      });
      setFieldError(null);
      setConfirm(null);
      void send(attempt);
    } catch {
      setFieldError("Check the Pipeline ID, graph, and current revisions.");
    }
  }
  async function refresh() {
    const fresh = await baseline.refetch();
    if (fresh.isError || !fresh.data) {
      setFieldError(describeApiError(fresh.error));
      return;
    }
    setAcknowledgedRevision(fresh.data.current.revision);
    setFieldError(null);
    setConfirm(null);
    setStatus({ kind: "editing" });
  }
  const row = baseline.data?.current;
  return (
    <section
      aria-label={`Manage Pipeline ${item.id}`}
      className="space-y-4 rounded-xl border border-border bg-card p-5"
    >
      <h3 ref={titleRef} tabIndex={-1} className="font-semibold">
        Manage {item.name}
      </h3>
      <p className="text-xs text-muted-foreground">
        Publishing creates a new immutable version. Default changes affect future Tasks; existing
        pinned Tasks keep their version. Deletion retains history.
      </p>
      {baseline.isPending && <p role="status">Loading Project and Pipeline revisions…</p>}
      {baseline.isError && <p role="alert">{describeApiError(baseline.error)}</p>}
      {row && (
        <p>
          Catalog revision {row.revision}; Project revision {baseline.data?.project.revision}.
        </p>
      )}
      {row && row.revision !== acknowledgedRevision && (
        <p role="alert">
          Catalog changed since this editor opened. Your graph text remains here. Refresh the
          baseline before sending.
        </p>
      )}
      {row && !row.deleted_at && status.kind === "editing" && (
        <div className="space-y-3">
          <label className="block text-sm">
            Source version ID for a new publication
            <Input value={sourceId} onChange={(event) => setSourceId(event.target.value)} />
          </label>
          <Button
            variant="outline"
            disabled={
              !source.data || source.data.pipeline_id.toLowerCase() !== item.id.toLowerCase()
            }
            onClick={() => {
              if (source.data)
                setDefinitionText(JSON.stringify(definitionFromVersion(source.data), null, 2));
            }}
          >
            Copy source version graph into editor
          </Button>
          {source.isError && <p role="alert">{describeApiError(source.error, "Pipeline")}</p>}
          <label className="block text-sm">
            Complete new graph JSON
            <textarea
              value={definitionText}
              onChange={(event) => setDefinitionText(event.target.value)}
              rows={12}
              className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
            />
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={makeDefault}
              onChange={(event) => setMakeDefault(event.target.checked)}
            />
            Make the new version default for future Tasks
          </label>
          <label className="block text-sm">
            Existing version ID to make default
            <Input value={defaultId} onChange={(event) => setDefaultId(event.target.value)} />
          </label>
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => setConfirm("publish_pipeline_version")}>
              Publish new version
            </Button>
            <Button variant="outline" onClick={() => setConfirm("set_pipeline_default_version")}>
              Set default version
            </Button>
            <Button variant="destructive" onClick={() => setConfirm("delete_pipeline")}>
              Delete Pipeline
            </Button>
          </div>
        </div>
      )}
      {confirm && (
        <div
          role="alertdialog"
          aria-labelledby="pipeline-confirm-title"
          className="space-y-2 rounded border border-border p-3"
        >
          <h4 id="pipeline-confirm-title" tabIndex={-1} ref={confirmRef}>
            Confirm{" "}
            {confirm === "publish_pipeline_version"
              ? "new immutable publication"
              : confirm === "delete_pipeline"
                ? "soft deletion"
                : "default version change"}
          </h4>
          <p className="text-sm">This creates a new catalog revision.</p>
          <Button onClick={() => void prepare(confirm)}>Confirm</Button>
          <Button variant="outline" onClick={() => setConfirm(null)}>
            Cancel
          </Button>
        </div>
      )}
      {fieldError && <p role="alert">{fieldError}</p>}
      {status.kind === "sending" && <p role="status">Sending Pipeline command…</p>}
      {status.kind === "unknown" && (
        <div className="space-y-2">
          <p role="alert">Outcome unknown. Retry the exact same request to obtain its receipt.</p>
          <Button onClick={() => void send(status.attempt)}>Retry same request</Button>
        </div>
      )}
      {status.kind === "refused" && (
        <div className="space-y-2">
          <p role="alert">
            {status.error instanceof LiveCommandError
              ? `Core refused this change (${status.error.kind}). Refresh and review the graph.`
              : "The request could not be confirmed."}
          </p>
          <Button onClick={() => void refresh()}>Refresh baseline</Button>
        </div>
      )}
      {status.kind === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Command accepted. {status.readError ? "Catalog readback failed." : "Reading catalog…"}
          </p>
          {status.readError && (
            <Button onClick={() => void readAccepted(status.attempt, status.receiptId)}>
              Retry catalog read
            </Button>
          )}
        </div>
      )}
      <Button
        variant="outline"
        onClick={() => {
          if (leaveGuard.canLeave()) onClose();
        }}
      >
        Close editor
      </Button>
    </section>
  );
}
