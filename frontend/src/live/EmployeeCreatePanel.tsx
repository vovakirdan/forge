import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  CreateEmployeeRequestSchema,
  createEmployeeAttempt,
  type CreateEmployeeAttempt,
  type EmployeeCommandReceipt,
} from "../contracts/create-employee.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { readKeys } from "./read-cache.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type StageChoice = { pipeline_version_id: string; stage_id: string; label: string };
type Props = ProjectReadScope & { onCancel: () => void; onCreated: (id: string) => void };
type State =
  | { status: "editing" }
  | { status: "sending" | "unknown"; attempt: CreateEmployeeAttempt }
  | { status: "refused"; error: LiveCommandError }
  | { status: "created"; receipt: EmployeeCommandReceipt; readError: boolean };

function describeRefusal(error: LiveCommandError) {
  switch (error.kind) {
    case "stale_revision":
      return "Project changed. Refresh the creation baseline before retrying.";
    case "idempotency_conflict":
      return "This request key belongs to another command. Refresh the creation baseline.";
    case "validation_failed":
      return "Core refused this Employee configuration. Check the fields and refresh the baseline.";
    case "not_found":
      return "The Project or selected Pipeline version was not found. Refresh the baseline.";
    case "forbidden":
      return "Core refused Employee creation. Refresh the baseline before retrying.";
    case "invalid_request":
      return "The request was invalid. Check the fields and refresh the baseline.";
    case "outcome_unknown":
      return "Creation outcome unknown. Retry the same request to obtain its receipt.";
  }
}

export function EmployeeCreatePanel({ onCancel, onCreated, ...scope }: Props) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [name, setName] = useState("");
  const [role, setRole] = useState("");
  const [mode, setMode] = useState<"" | "any" | "only">("");
  const [selected, setSelected] = useState<StageChoice[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [state, setState] = useState<State>({ status: "editing" });
  const [refreshing, setRefreshing] = useState(false);
  const [baselineError, setBaselineError] = useState<string | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const nameInput = useRef<HTMLInputElement>(null);
  const projectKey = useMemo(
    () => ["project", generation, projectId] as const,
    [generation, projectId],
  );
  const project = useQuery({
    queryKey: projectKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.project(projectId, token, signal)),
    retry: false,
  });
  const versionsKey = useMemo(
    () => readKeys.pipelineVersions(generation, projectId, cursor),
    [generation, projectId, cursor],
  );
  const versions = useQuery({
    queryKey: versionsKey,
    enabled: mode === "only",
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.pipelines(projectId, cursor, token, signal)),
    retry: false,
  });
  useReadLifetime(versionsKey);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    nameInput.current?.focus();
    return () => controller.abort();
  }, []);
  const dirty = name !== "" || role !== "" || mode !== "" || selected.length > 0;
  const guard =
    state.status === "sending" ||
    state.status === "unknown" ||
    (state.status !== "created" && dirty);
  useEffect(() => {
    if (!guard) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Employee creation? Local fields and the retry key will be lost. An in-flight or unknown creation may still have been applied by Core; leaving does not undo it.",
    );
    function beforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "";
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [leaveGuard, guard]);

  function stillCurrent(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }
  function navigate(next: string | null, history: (string | null)[]) {
    setCursor(next);
    setPrevious(history);
  }
  function toggle(choice: StageChoice) {
    setSelected((current) =>
      current.some(
        (item) =>
          item.pipeline_version_id === choice.pipeline_version_id &&
          item.stage_id === choice.stage_id,
      )
        ? current.filter(
            (item) =>
              item.pipeline_version_id !== choice.pipeline_version_id ||
              item.stage_id !== choice.stage_id,
          )
        : [...current, choice],
    );
  }
  const input =
    project.data && mode !== ""
      ? {
          project_id: projectId,
          expected_revision: project.data.revision,
          payload: {
            name,
            role,
            stage_eligibility:
              mode === "any"
                ? { mode: "any" as const }
                : {
                    mode: "only" as const,
                    stages: selected.map(({ pipeline_version_id, stage_id }) => ({
                      pipeline_version_id,
                      stage_id,
                    })),
                  },
          },
        }
      : null;
  const canCreate =
    state.status === "editing" &&
    !project.isFetching &&
    !project.isError &&
    (mode !== "only" || !versions.isError) &&
    input !== null &&
    CreateEmployeeRequestSchema.safeParse(input).success;

  async function readCreated(receipt: EmployeeCommandReceipt, controller: AbortController) {
    setState({ status: "created", receipt, readError: false });
    try {
      const [freshProject, employee] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
        session.request(generation, (token) =>
          api.employee(projectId, receipt.resource.id, token, controller.signal),
        ),
      ]);
      if (!stillCurrent(controller)) return;
      if (freshProject.revision < receipt.project_revision)
        throw new Error("Stale Project readback");
      await Promise.all([
        queries.cancelQueries({ queryKey: projectKey, exact: true }),
        queries.cancelQueries({
          queryKey: readKeys.employee(generation, projectId, employee.id),
          exact: true,
        }),
      ]);
      if (!stillCurrent(controller)) return;
      queries.setQueryData(projectKey, freshProject);
      queries.setQueryData(readKeys.employee(generation, projectId, employee.id), employee);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "employees"] },
        { throwOnError: true },
      );
      if (stillCurrent(controller)) onCreated(employee.id);
    } catch {
      if (stillCurrent(controller)) setState({ status: "created", receipt, readError: true });
    }
  }
  async function submit(attempt: CreateEmployeeAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.createEmployee(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (stillCurrent(controller)) await readCreated(receipt, controller);
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
    if (!controller || refreshing) return;
    setRefreshing(true);
    setBaselineError(null);
    try {
      const currentProject = await project.refetch();
      if (currentProject.isError || !currentProject.data) throw currentProject.error;
      if (mode === "only") {
        const currentVersions = await versions.refetch();
        if (currentVersions.isError) throw currentVersions.error;
      }
      if (stillCurrent(controller)) setState({ status: "editing" });
    } catch (error) {
      if (stillCurrent(controller)) setBaselineError(describeApiError(error));
    } finally {
      if (stillCurrent(controller)) setRefreshing(false);
    }
  }
  const disabled = state.status !== "editing";
  return (
    <section
      aria-label="Create Employee"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <h3 className="font-semibold">Create Employee</h3>
      <p className="text-sm text-muted-foreground">
        A new Employee starts enabled. Creation does not request onboarding or configure a runtime.
      </p>
      {project.isPending && <p role="status">Loading Project revision…</p>}
      {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
      {project.isError && (
        <Button variant="outline" onClick={() => void project.refetch()}>
          Retry Project read
        </Button>
      )}
      <form
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (canCreate && input)
            void submit(createEmployeeAttempt(CreateEmployeeRequestSchema.parse(input)));
        }}
      >
        <label className="block space-y-1 text-sm font-medium">
          Employee name
          <Input
            ref={nameInput}
            value={name}
            maxLength={200}
            disabled={disabled}
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label className="block space-y-1 text-sm font-medium">
          Role
          <Input
            value={role}
            maxLength={128}
            disabled={disabled}
            onChange={(event) => setRole(event.target.value)}
          />
        </label>
        <fieldset className="space-y-2" disabled={disabled}>
          <legend className="text-sm font-medium">Stage eligibility</legend>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name="eligibility"
              checked={mode === "any"}
              onChange={() => setMode("any")}
            />
            Any employee stage
          </label>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name="eligibility"
              checked={mode === "only"}
              onChange={() => setMode("only")}
            />
            Only selected stages
          </label>
        </fieldset>
        {mode === "only" && (
          <div className="space-y-3 rounded-lg border border-border p-3">
            <p className="text-sm">Selected: {selected.length} of 128 stage targets</p>
            {versions.isPending && <p role="status">Loading Pipeline versions…</p>}
            {versions.isError && <p role="alert">{describeApiError(versions.error, "Pipeline")}</p>}
            {versions.isError && (
              <Button type="button" variant="outline" onClick={() => void versions.refetch()}>
                Retry Pipeline versions
              </Button>
            )}
            {versions.data?.items
              .filter((version) => version.deleted_at === null)
              .flatMap((version) =>
                version.stages
                  .filter((stage) => stage.executor_kind === "employee")
                  .map((stage) => {
                    const choice = {
                      pipeline_version_id: version.id,
                      stage_id: stage.id,
                      label: `${version.name} v${version.version}: ${stage.name}`,
                    };
                    const checked = selected.some(
                      (item) =>
                        item.pipeline_version_id === choice.pipeline_version_id &&
                        item.stage_id === choice.stage_id,
                    );
                    return (
                      <label
                        key={`${version.id}:${stage.id}`}
                        className="flex items-start gap-2 text-sm [overflow-wrap:anywhere]"
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          disabled={disabled || (!checked && selected.length >= 128)}
                          onChange={() => toggle(choice)}
                        />
                        {choice.label}
                      </label>
                    );
                  }),
              )}
            {versions.data?.items.length === 0 && (
              <p className="text-sm">No Pipeline versions on this page.</p>
            )}
            <nav
              aria-label="Eligible stage pages"
              className="flex items-center justify-between gap-2"
            >
              <Button
                type="button"
                variant="outline"
                disabled={previous.length === 0 || versions.isFetching}
                onClick={() => navigate(previous.at(-1) ?? null, previous.slice(0, -1))}
              >
                Previous versions
              </Button>
              <span className="text-sm">Page {previous.length + 1}</span>
              <Button
                type="button"
                variant="outline"
                disabled={!versions.data?.next_cursor || versions.isFetching}
                onClick={() => {
                  if (versions.data?.next_cursor)
                    navigate(versions.data.next_cursor, [...previous, cursor]);
                }}
              >
                Next versions
              </Button>
            </nav>
          </div>
        )}
        {state.status === "refused" && <p role="alert">{describeRefusal(state.error)}</p>}
        {state.status === "unknown" && (
          <p role="alert">
            Creation outcome unknown. Retry the same request to obtain its receipt.
          </p>
        )}
        {state.status === "created" && (
          <p role="status">
            Employee created. Receipt {state.receipt.command_id}.{" "}
            {state.readError
              ? "Profile readback failed; retry the read."
              : "Loading canonical profile…"}
          </p>
        )}
        {baselineError && <p role="alert">{baselineError}</p>}
        <div className="flex flex-wrap gap-2">
          {state.status === "editing" && (
            <Button type="submit" disabled={!canCreate}>
              Create Employee
            </Button>
          )}
          {state.status === "sending" && (
            <Button type="button" disabled>
              Creating…
            </Button>
          )}
          {state.status === "unknown" && (
            <Button
              type="button"
              disabled={inFlight.current}
              onClick={() => void submit(state.attempt)}
            >
              Retry same creation
            </Button>
          )}
          {state.status === "refused" && (
            <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
              Refresh creation baseline
            </Button>
          )}
          {state.status === "created" && state.readError && (
            <Button
              type="button"
              onClick={() => {
                const controller = lifetime.current;
                if (controller) void readCreated(state.receipt, controller);
              }}
            >
              Retry profile readback
            </Button>
          )}
          {state.status !== "sending" && state.status !== "unknown" && (
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                if (leaveGuard.canLeave()) onCancel();
              }}
            >
              Close creation
            </Button>
          )}
        </div>
      </form>
    </section>
  );
}
