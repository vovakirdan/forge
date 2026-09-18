import { startTransition, useActionState, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { Textarea } from "../components/ui/textarea.tsx";
import {
  DraftTitleSchema,
  DraftDescriptionSchema,
  DraftDefinitionOfDoneSchema,
} from "../contracts/amend-draft.ts";
import type { AmendDraftReceipt } from "../contracts/amend-draft.ts";
import { describeApiError } from "./api.ts";
import { describeCommandError, LiveCommandError } from "./command-error.ts";
import {
  createDraftAttempt,
  draftChanged,
  rebaseDraftFields,
  normalizeDraftDoD,
} from "./draft-attempt.ts";
import type { DraftAttempt, DraftBaseline, DraftFields } from "./draft-attempt.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type EditorProps = ProjectReadScope & { taskId: string; onCancel: () => void };

async function readBaseline(scope: EditorProps, signal: AbortSignal): Promise<DraftBaseline> {
  const { session, generation, api, projectId, taskId } = scope;
  const [project, task] = await Promise.all([
    session.request(generation, (token) => api.project(projectId, token, signal)),
    session.request(generation, (token) => api.task(projectId, taskId, token, signal)),
  ]);
  return { project, task };
}

export function TaskDraftEditor(scope: EditorProps) {
  const key = useMemo(
    () => ["live", scope.generation, scope.projectId, "draft-baseline", scope.taskId] as const,
    [scope.generation, scope.projectId, scope.taskId],
  );
  const baseline = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => readBaseline(scope, signal),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section aria-label="Edit draft" className="space-y-3 rounded-lg border border-border p-4">
      <h3 className="font-semibold">Edit draft</h3>
      {baseline.isPending && <p role="status">Loading current edit baseline…</p>}
      {baseline.isError && (
        <p role="alert">
          Cannot load the edit baseline. {describeApiError(baseline.error, "Task")}
        </p>
      )}
      {baseline.isError && (
        <Button onClick={() => void baseline.refetch()} disabled={baseline.isFetching}>
          Retry edit baseline
        </Button>
      )}
      {baseline.data ? (
        <DraftForm {...scope} initial={baseline.data} />
      ) : (
        <Button variant="outline" onClick={scope.onCancel}>
          Cancel
        </Button>
      )}
    </section>
  );
}

type SaveState =
  | { status: "editing" }
  | { status: "sending" | "unknown"; attempt: DraftAttempt }
  | { status: "refused"; error: LiveCommandError }
  | { status: "saved"; receipt: AmendDraftReceipt; refreshing: boolean; refreshFailed: boolean };

function DraftForm(scope: EditorProps & { initial: DraftBaseline }) {
  const { api, session, generation, projectId, taskId, leaveGuard, onCancel } = scope;
  const queries = useQueryClient();
  const [baseline, setBaseline] = useState(scope.initial);
  const [fields, setFields] = useState<DraftFields>({
    title: baseline.task.title,
    description: baseline.task.description,
    definition_of_done: baseline.task.definition_of_done ?? "",
  });
  const [state, setState] = useState<SaveState>({ status: "editing" });
  const [refreshing, setRefreshing] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [baselineRefreshed, setBaselineRefreshed] = useState(false);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  const changed = draftChanged(fields, baseline.task);
  const warn =
    state.status === "sending" ||
    state.status === "unknown" ||
    (state.status !== "saved" && changed);
  useEffect(() => {
    if (!warn) return;
    const unregister = leaveGuard.register(() => true);
    function beforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "";
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [leaveGuard, warn]);

  function stillCurrent(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }

  async function refreshSaved(receipt: AmendDraftReceipt, controller: AbortController) {
    setState({ status: "saved", receipt, refreshing: true, refreshFailed: false });
    try {
      const current = await readBaseline(scope, controller.signal);
      if (!stillCurrent(controller)) return;
      await Promise.all([
        queries.cancelQueries({ queryKey: ["project", generation, projectId], exact: true }),
        queries.cancelQueries({
          queryKey: readKeys.task(generation, projectId, taskId),
          exact: true,
        }),
      ]);
      if (!stillCurrent(controller)) return;
      queries.setQueryData(["project", generation, projectId], current.project);
      queries.setQueryData(readKeys.task(generation, projectId, taskId), current.task);
      setBaseline(current);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "tasks"] },
        { throwOnError: true },
      );
      if (stillCurrent(controller))
        setState({ status: "saved", receipt, refreshing: false, refreshFailed: false });
    } catch {
      if (stillCurrent(controller))
        setState({ status: "saved", receipt, refreshing: false, refreshFailed: true });
    }
  }

  async function submit(attempt: DraftAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.amendDraft(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (stillCurrent(controller)) await refreshSaved(receipt, controller);
    } catch (error) {
      if (!stillCurrent(controller)) return;
      setState(
        error instanceof LiveCommandError && error.kind !== "outcome_unknown"
          ? { status: "refused", error }
          : { status: "unknown", attempt },
      );
    } finally {
      inFlight.current = false;
    }
  }

  async function refreshBaseline() {
    const controller = lifetime.current;
    if (!controller || refreshing || state.status !== "refused") return;
    setRefreshing(true);
    setReadError(null);
    try {
      const current = await readBaseline(scope, controller.signal);
      if (!stillCurrent(controller)) return;
      setFields((previous) => rebaseDraftFields(previous, baseline.task, current.task));
      setBaseline(current);
      setBaselineRefreshed(true);
      setState({ status: "editing" });
    } catch (error) {
      if (stillCurrent(controller)) setReadError(describeApiError(error, "Task"));
    } finally {
      if (stillCurrent(controller)) setRefreshing(false);
    }
  }
  const titleValid = DraftTitleSchema.safeParse(fields.title).success;
  const descriptionValid = DraftDescriptionSchema.safeParse(fields.description).success;
  const dodValid = DraftDefinitionOfDoneSchema.nullable().safeParse(
    normalizeDraftDoD(fields.definition_of_done),
  ).success;
  const revisionSupported =
    baseline.project.revision < Number.MAX_SAFE_INTEGER &&
    baseline.task.revision < Number.MAX_SAFE_INTEGER;
  const editable =
    state.status === "editing" && baseline.task.lifecycle === "draft" && revisionSupported;
  const canSave = editable && changed && titleValid && descriptionValid && dodValid;
  const [, save, actionPending] = useActionState(async (_previous: null) => {
    if (canSave) await submit(createDraftAttempt(baseline, fields));
    return null;
  }, null);

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        startTransition(save);
      }}
      className="space-y-3"
    >
      <p className="text-xs text-muted-foreground">
        Only title, description and definition of done change. This Task stays a draft; saving does
        not approve it.
      </p>
      <p className="text-xs">
        Edit baseline: Project revision {baseline.project.revision}, Task revision{" "}
        {baseline.task.revision}.
      </p>
      {baselineRefreshed && (
        <details className="space-y-2 rounded border border-border p-3">
          <summary>Current saved values</summary>
          <dl className="space-y-2 text-sm">
            <dt>Saved title</dt>
            <dd className="whitespace-pre-wrap [overflow-wrap:anywhere]">{baseline.task.title}</dd>
            <dt>Saved description</dt>
            <dd className="whitespace-pre-wrap [overflow-wrap:anywhere]">
              {baseline.task.description || "No description."}
            </dd>
            <dt>Saved definition of done</dt>
            <dd className="whitespace-pre-wrap [overflow-wrap:anywhere]">
              {baseline.task.definition_of_done ?? "Not provided."}
            </dd>
          </dl>
          <p className="text-xs">
            Your changed fields remain below. Saving explicitly replaces those fields.
          </p>
        </details>
      )}
      {baseline.task.lifecycle !== "draft" && (
        <p role="alert">
          This Task is no longer a draft. Editing is unavailable; your local text is retained below.
        </p>
      )}
      {!revisionSupported && (
        <p role="alert">
          The revision exceeds the browser edit limit. This draft cannot be saved here.
        </p>
      )}
      <div className="space-y-2">
        <label htmlFor="draft-title">Draft title</label>
        <Input
          id="draft-title"
          value={fields.title}
          disabled={!editable || actionPending}
          onChange={(event) => setFields({ ...fields, title: event.target.value })}
          aria-invalid={!titleValid}
          aria-describedby="draft-title-help"
        />
        <p id="draft-title-help" className="text-xs">
          Required; at most 240 Unicode characters.{" "}
          {titleValid ? "" : "Enter a nonblank title within the limit."}
        </p>
      </div>
      <div className="space-y-2">
        <label htmlFor="draft-description">Draft description</label>
        <Textarea
          id="draft-description"
          value={fields.description}
          disabled={!editable || actionPending}
          onChange={(event) => setFields({ ...fields, description: event.target.value })}
          aria-invalid={!descriptionValid}
          aria-describedby="draft-description-help"
        />
        <p id="draft-description-help" className="text-xs">
          Optional; at most 50,000 Unicode characters.{" "}
          {descriptionValid ? "" : "Description exceeds the limit or contains invalid Unicode."}
        </p>
      </div>
      <div className="space-y-2">
        <label htmlFor="draft-dod">Draft definition of done</label>
        <Textarea
          id="draft-dod"
          value={fields.definition_of_done ?? ""}
          disabled={!editable || actionPending}
          onChange={(event) => setFields({ ...fields, definition_of_done: event.target.value })}
          aria-invalid={!dodValid}
          aria-describedby="draft-dod-help"
        />
        <p id="draft-dod-help" className="text-xs">
          Optional for a draft; blank clears the saved value. At most 20,000 Unicode characters.{" "}
          {dodValid ? "" : "Definition of done exceeds the limit or contains invalid Unicode."}
        </p>
      </div>
      {state.status === "sending" && <p role="status">Saving draft…</p>}
      {state.status === "unknown" && (
        <>
          <p role="alert">{describeCommandError(new LiveCommandError("outcome_unknown"))}</p>
          <Button type="button" disabled={actionPending} onClick={() => void submit(state.attempt)}>
            Retry same save
          </Button>
        </>
      )}
      {state.status === "refused" && (
        <>
          <p role="alert">{describeCommandError(state.error)}</p>
          <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
            Refresh edit baseline
          </Button>
        </>
      )}
      {readError && <p role="alert">{readError} Local text has been retained.</p>}
      {state.status === "saved" && (
        <>
          <p role="status">
            Saved.{" "}
            {state.refreshing
              ? "Refreshing authoritative data…"
              : state.refreshFailed
                ? "Could not refresh authoritative data. The save is confirmed."
                : "Authoritative data refreshed."}
          </p>
          <p className="text-xs">
            Receipt: {state.receipt.command_id} ({state.receipt.status})
          </p>
          {state.refreshFailed && (
            <Button
              type="button"
              onClick={() => {
                const controller = lifetime.current;
                if (controller) void refreshSaved(state.receipt, controller);
              }}
            >
              Refresh saved data
            </Button>
          )}
        </>
      )}
      <div className="flex gap-2">
        {state.status !== "saved" && (
          <Button type="submit" disabled={!canSave || actionPending}>
            Save
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            if (leaveGuard.canLeave()) onCancel();
          }}
        >
          {state.status === "saved" ? "Close editor" : "Cancel"}
        </Button>
      </div>
    </form>
  );
}
