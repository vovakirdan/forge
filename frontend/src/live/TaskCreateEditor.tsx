import { startTransition, useActionState, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { Textarea } from "../components/ui/textarea.tsx";
import {
  CreateTaskRequestSchema,
  DraftDefinitionOfDoneSchema,
  type CreateTaskAttempt,
} from "../contracts/create-task.ts";
import { DraftTitleSchema, DraftDescriptionSchema } from "../contracts/amend-draft.ts";
import type { TaskCommandReceipt } from "../contracts/task-command.ts";
import type { PrioritySchemeView } from "../contracts/priority-scheme.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { describeApiError } from "./api.ts";
import { describeCommandError, LiveCommandError } from "./command-error.ts";
import { activePriority } from "./priority-attempt.ts";
import {
  createTaskAttempt,
  createTaskInput,
  initialCreateTaskFields,
  pipelineAllowsCreation,
} from "./create-task-attempt.ts";
import { readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { CreateTaskPipelinePicker } from "./CreateTaskPipelinePicker.tsx";
import { TaskPropertiesSchema } from "../contracts/properties.ts";

type Props = ProjectReadScope & { onCancel: () => void; onCreated: (taskId: string) => void };
export function TaskCreateEditor(scope: Props) {
  const { session, generation, projectId, api } = scope;
  const key = useMemo(
    () => ["live", generation, projectId, "create-baseline"] as const,
    [generation, projectId],
  );
  const baseline = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.priorityScheme(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section
      aria-label="Create draft Task"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <h3 className="font-semibold">Create draft Task</h3>
      {baseline.isPending && <p role="status">Loading creation baseline…</p>}
      {baseline.isError && (
        <>
          <p role="alert">Creation baseline unavailable. {describeApiError(baseline.error)}</p>
          <Button onClick={() => void baseline.refetch()} disabled={baseline.isFetching}>
            Retry creation baseline
          </Button>
        </>
      )}
      {baseline.data ? (
        <CreateForm {...scope} initial={baseline.data} />
      ) : (
        <Button variant="outline" onClick={scope.onCancel}>
          Cancel
        </Button>
      )}
    </section>
  );
}
type State =
  | { status: "editing" }
  | { status: "sending" | "unknown"; attempt: CreateTaskAttempt }
  | { status: "refused"; error: LiveCommandError }
  | { status: "created"; receipt: TaskCommandReceipt; refreshing: boolean; refreshFailed: boolean };

function CreateForm(scope: Props & { initial: PrioritySchemeView }) {
  const { api, session, generation, projectId, leaveGuard, onCancel, onCreated } = scope;
  const queries = useQueryClient();
  const [scheme, setScheme] = useState(scope.initial);
  const [fields, setFields] = useState(() => initialCreateTaskFields(scope.initial));
  const [propertiesText, setPropertiesText] = useState("{}");
  const [state, setState] = useState<State>({ status: "editing" });
  const [refreshing, setRefreshing] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  const dirty =
    fields.title !== "" ||
    fields.description !== "" ||
    fields.definitionOfDone !== "" ||
    fields.kind !== "" ||
    fields.pipelineVersionId !== "" ||
    propertiesText !== "{}" ||
    fields.priority !== scope.initial.default_level_id;
  const warn =
    state.status === "sending" ||
    state.status === "unknown" ||
    (state.status !== "created" && dirty);
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
  const pipelineKey = useMemo(
    () =>
      [
        "live",
        generation,
        projectId,
        "create-selected-pipeline",
        fields.pipelineVersionId,
      ] as const,
    [generation, projectId, fields.pipelineVersionId],
  );
  const pipeline = useQuery({
    queryKey: pipelineKey,
    enabled: fields.pipelineVersionId !== "",
    retry: false,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.pipeline(projectId, fields.pipelineVersionId, token, signal),
      ),
  });
  useReadLifetime(pipelineKey);
  const schemaKey = useMemo(
    () => readKeys.taskPropertySchema(generation, projectId),
    [generation, projectId],
  );
  const propertySchema = useQuery({
    queryKey: schemaKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.taskPropertySchema(projectId, token, signal)),
    retry: false,
  });
  async function refreshCreated(receipt: TaskCommandReceipt, controller: AbortController) {
    setState({ status: "created", receipt, refreshing: true, refreshFailed: false });
    try {
      const [project, task] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
        session.request(generation, (token) =>
          api.task(projectId, receipt.resource.id, token, controller.signal),
        ),
      ]);
      if (!stillCurrent(controller)) return;
      const projectKey = ["project", generation, projectId] as const;
      const taskKey = readKeys.task(generation, projectId, receipt.resource.id);
      await Promise.all(
        [projectKey, taskKey].map((queryKey) => queries.cancelQueries({ queryKey, exact: true })),
      );
      if (!stillCurrent(controller)) return;
      queries.setQueryData(projectKey, project);
      queries.setQueryData(taskKey, task);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "tasks"] },
        { throwOnError: true },
      );
      if (stillCurrent(controller)) onCreated(receipt.resource.id);
    } catch {
      if (stillCurrent(controller))
        setState({ status: "created", receipt, refreshing: false, refreshFailed: true });
    }
  }
  async function submit(attempt: CreateTaskAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.createTask(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (stillCurrent(controller)) await refreshCreated(receipt, controller);
    } catch (error) {
      if (stillCurrent(controller))
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
      const current = await session.request(generation, (token) =>
        api.priorityScheme(projectId, token, controller.signal),
      );
      if (fields.pipelineVersionId !== "") await pipeline.refetch();
      if (!stillCurrent(controller)) return;
      setScheme(current);
      setState({ status: "editing" });
    } catch (error) {
      if (stillCurrent(controller)) setReadError(describeApiError(error));
    } finally {
      if (stillCurrent(controller)) setRefreshing(false);
    }
  }
  const editable = state.status === "editing";
  const priorityValid = activePriority(scheme, fields.priority);
  const pipelineValid =
    pipeline.isSuccess && !pipeline.isFetching && pipelineAllowsCreation(pipeline.data, fields);
  let propertyValues: ReturnType<typeof TaskPropertiesSchema.safeParse>;
  try {
    propertyValues = TaskPropertiesSchema.safeParse(JSON.parse(propertiesText) as unknown);
  } catch {
    propertyValues = TaskPropertiesSchema.safeParse(null);
  }
  const propertyValid = propertyValues.success && Object.keys(propertyValues.data).length <= 64;
  const canCreate =
    editable &&
    priorityValid &&
    pipelineValid &&
    propertyValid &&
    CreateTaskRequestSchema.safeParse(createTaskInput(scheme, fields, propertyValues.data)).success;
  const [, create, pending] = useActionState(async (_previous: null) => {
    if (canCreate && pipeline.data && propertyValues.success)
      await submit(
        createTaskAttempt(scheme, pipeline.data, fields, undefined, propertyValues.data),
      );
    return null;
  }, null);
  const disabled = !editable || pending;
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        startTransition(create);
      }}
      className="space-y-4"
    >
      <p className="text-sm">Creates a draft only. No approval, queue entry or Run is requested.</p>
      <p className="text-xs">Creation baseline: Project revision {scheme.project_revision}.</p>
      {scheme.project_revision === Number.MAX_SAFE_INTEGER && (
        <p role="alert">The revision exceeds the browser creation limit.</p>
      )}
      <div className="space-y-2">
        <label htmlFor="create-title">Draft title</label>
        <Input
          id="create-title"
          value={fields.title}
          disabled={disabled}
          onChange={(event) => setFields({ ...fields, title: event.target.value })}
          aria-invalid={!DraftTitleSchema.safeParse(fields.title).success}
        />
        <p className="text-xs">Required; at most 240 Unicode characters.</p>
      </div>
      <div className="space-y-2">
        <label htmlFor="create-properties">Task properties JSON</label>
        <Textarea
          id="create-properties"
          className="font-mono text-xs"
          value={propertiesText}
          disabled={disabled}
          aria-invalid={!propertyValid}
          onChange={(event) => setPropertiesText(event.target.value)}
        />
        {propertySchema.data && (
          <p className="text-xs">
            Project schema:{" "}
            {Object.values(propertySchema.data.schema.definitions)
              .map(
                (definition) =>
                  `${definition.key} (${definition.property_type}${definition.required ? ", required" : ""})`,
              )
              .join(", ") || "no configured fields"}
            . Values use tagged objects such as {`{"impact":{"type":"enum","value":"high"}}`}.
          </p>
        )}
        {propertySchema.isError && (
          <p role="alert">
            Property definitions unavailable. Core will validate submitted values at approval.
          </p>
        )}
        {!propertyValid && (
          <p role="alert">Enter at most 64 stable property keys with tagged values.</p>
        )}
      </div>
      <div className="space-y-2">
        <label htmlFor="create-description">Draft description</label>
        <Textarea
          id="create-description"
          value={fields.description}
          disabled={disabled}
          onChange={(event) => setFields({ ...fields, description: event.target.value })}
          aria-invalid={!DraftDescriptionSchema.safeParse(fields.description).success}
        />
        <p className="text-xs">Optional; at most 50,000 Unicode characters.</p>
      </div>
      <div className="space-y-2">
        <label htmlFor="create-dod">Definition of done</label>
        <Textarea
          id="create-dod"
          value={fields.definitionOfDone}
          disabled={disabled}
          onChange={(event) => setFields({ ...fields, definitionOfDone: event.target.value })}
          aria-invalid={
            fields.definitionOfDone !== "" &&
            !DraftDefinitionOfDoneSchema.safeParse(fields.definitionOfDone).success
          }
        />
        <p className="text-xs">
          Optional for a draft; leave empty to omit. Otherwise nonblank, at most 20,000 Unicode
          characters.
        </p>
      </div>
      <div className="space-y-2">
        <label htmlFor="create-kind">Task kind</label>
        <select
          id="create-kind"
          className="block w-full min-w-0 rounded border border-border bg-background p-2"
          value={fields.kind}
          disabled={disabled}
          onChange={(event) => {
            const kind = event.target.value;
            if (kind === "delivery" || kind === "analysis") setFields({ ...fields, kind });
          }}
        >
          <option value="" disabled>
            Choose kind
          </option>
          <option value="delivery">Delivery</option>
          <option value="analysis">Analysis</option>
        </select>
      </div>
      <div className="space-y-2">
        <label htmlFor="create-priority">Task priority</label>
        <select
          id="create-priority"
          className="block w-full min-w-0 rounded border border-border bg-background p-2"
          value={fields.priority}
          disabled={disabled}
          onChange={(event) => setFields({ ...fields, priority: event.target.value })}
          aria-invalid={!priorityValid}
        >
          {!priorityValid && (
            <option value={fields.priority} disabled>
              {fields.priority} (unavailable; choose an active priority)
            </option>
          )}
          {scheme.levels
            .filter((level) => !level.retired)
            .map((level) => (
              <option key={level.id} value={level.id}>
                {level.display_name} ({level.id})
              </option>
            ))}
        </select>
        {!priorityValid && (
          <p role="alert">
            Selected priority is unavailable or retired. Choose an active priority.
          </p>
        )}
      </div>
      <CreateTaskPipelinePicker
        {...scope}
        selected={fields.pipelineVersionId}
        disabled={disabled}
        onSelect={(pipelineVersionId) => setFields({ ...fields, pipelineVersionId })}
      />
      <div className="space-y-2 [overflow-wrap:anywhere]">
        <p>Selected Pipeline version: {fields.pipelineVersionId || "None selected"}</p>
        {fields.pipelineVersionId !== "" && pipeline.isFetching && (
          <p role="status">Checking selected Pipeline version…</p>
        )}
        {fields.pipelineVersionId !== "" && pipeline.isError && (
          <>
            <p role="alert">
              Selected Pipeline version unavailable. {describeApiError(pipeline.error, "Pipeline")}
            </p>
            <Button
              type="button"
              disabled={disabled || pipeline.isFetching}
              onClick={() => void pipeline.refetch()}
            >
              Retry selected Pipeline
            </Button>
          </>
        )}
        {pipeline.isSuccess && (
          <>
            <p>
              {pipeline.data.name} · version {pipeline.data.version}
            </p>
            {pipeline.data.deleted_at !== null && (
              <p role="alert">Selected Pipeline is deleted. Choose another version.</p>
            )}
            {fields.kind !== "" && !pipeline.data.task_kinds.includes(fields.kind) && (
              <p role="alert">Selected Pipeline version does not support this Task kind.</p>
            )}
          </>
        )}
      </div>
      {state.status === "sending" && <p role="status">Creating draft…</p>}
      {state.status === "unknown" && (
        <>
          <p role="alert">
            Creation outcome unknown. Core may have created the draft. Retry the same creation to
            obtain its receipt; do not create another draft.
          </p>
          <Button type="button" disabled={pending} onClick={() => void submit(state.attempt)}>
            Retry same creation
          </Button>
        </>
      )}
      {state.status === "refused" && (
        <>
          <p role="alert">{describeCommandError(state.error)}</p>
          <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
            Refresh creation baseline
          </Button>
        </>
      )}
      {readError && <p role="alert">{readError} All entered fields have been retained.</p>}
      {state.status === "created" && (
        <>
          <p role="status">
            Created Task: {state.receipt.resource.id}.{" "}
            {state.refreshing
              ? "Refreshing authoritative data…"
              : "Could not refresh authoritative data. Creation is confirmed."}
          </p>
          <p className="text-xs">
            Receipt: {state.receipt.command_id} ({state.receipt.status})
          </p>
          {state.refreshFailed && (
            <Button
              type="button"
              onClick={() => {
                const controller = lifetime.current;
                if (controller) void refreshCreated(state.receipt, controller);
              }}
            >
              Refresh created data
            </Button>
          )}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        {state.status !== "created" && (
          <Button type="submit" disabled={!canCreate || pending}>
            Create draft
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            if (leaveGuard.canLeave()) onCancel();
          }}
        >
          {state.status === "created" ? "Close creation" : "Cancel"}
        </Button>
      </div>
    </form>
  );
}
