import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  AmendEmployeeRequestSchema,
  amendEmployeeAttempt,
  type AmendEmployeeAttempt,
} from "../contracts/amend-employee.ts";
import type { EmployeeProfile } from "../contracts/employee.ts";
import type { EmployeeCommandReceipt } from "../contracts/create-employee.ts";
import { describeApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Stage = { pipeline_version_id: string; stage_id: string };
type State =
  | { status: "editing" }
  | { status: "sending" | "unknown"; attempt: AmendEmployeeAttempt }
  | { status: "refused"; error: LiveCommandError }
  | { status: "accepted"; receipt: EmployeeCommandReceipt; readError: boolean };

function targets(employee: EmployeeProfile): Stage[] {
  return employee.stage_eligibility.mode === "only" ? [...employee.stage_eligibility.stages] : [];
}

function sameTargets(left: Stage[], right: Stage[]) {
  const key = (stage: Stage) => `${stage.pipeline_version_id.toLowerCase()}:${stage.stage_id}`;
  return (
    left.length === right.length &&
    left.map(key).sort().join("|") === right.map(key).sort().join("|")
  );
}

function refusalMessage(error: LiveCommandError) {
  switch (error.kind) {
    case "stale_revision":
      return "Project or Employee changed. Refresh both edit baselines before saving again.";
    case "conflict":
      return "Core refused these changes in the current Employee state. Refresh both baselines.";
    case "validation_failed":
      return "Core refused these Employee changes. Check the fields and refresh the baselines.";
    case "idempotency_conflict":
      return "This request key belongs to another command. Refresh both edit baselines.";
    case "not_found":
      return "Employee or selected Pipeline version was not found. Refresh the baselines.";
    case "forbidden":
    case "invalid_request":
      return "Employee changes were refused. Check the fields and refresh the baselines.";
    case "outcome_unknown":
      return "Save outcome unknown. Retry the same request to obtain its receipt.";
  }
}

export function EmployeeEditPanel({
  scope,
  employee,
  onCancel,
  onSaved,
}: {
  scope: ProjectReadScope;
  employee: EmployeeProfile;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [baseline, setBaseline] = useState(employee);
  const [name, setName] = useState(employee.name);
  const [role, setRole] = useState(employee.role);
  const [capacity, setCapacity] = useState(String(employee.max_concurrent_runs));
  const [mode, setMode] = useState<"any" | "only">(employee.stage_eligibility.mode);
  const [selected, setSelected] = useState<Stage[]>(targets(employee));
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
  const profileKey = useMemo(
    () => readKeys.employee(generation, projectId, employee.id),
    [generation, projectId, employee.id],
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

  const eligibilityChanged =
    mode !== baseline.stage_eligibility.mode ||
    (mode === "only" && !sameTargets(selected, targets(baseline)));
  const patch = {
    ...(name.trim() !== baseline.name ? { name: name.trim() } : {}),
    ...(role.trim() !== baseline.role ? { role: role.trim() } : {}),
    ...(capacity !== String(baseline.max_concurrent_runs)
      ? { max_concurrent_runs: Number(capacity) }
      : {}),
    ...(eligibilityChanged
      ? {
          stage_eligibility:
            mode === "any"
              ? ({ mode: "any" } as const)
              : ({ mode: "only", stages: selected } as const),
        }
      : {}),
  };
  const input = project.data
    ? {
        project_id: projectId,
        expected_revision: project.data.revision,
        payload: {
          employee_id: employee.id,
          expected_employee_revision: baseline.revision,
          patch,
        },
      }
    : null;
  const dirty =
    name !== baseline.name ||
    role !== baseline.role ||
    capacity !== String(baseline.max_concurrent_runs) ||
    eligibilityChanged;
  const guard =
    state.status === "sending" ||
    state.status === "unknown" ||
    (state.status !== "accepted" && dirty);
  useEffect(() => {
    if (!guard) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Employee editing? Local changes and the retry key will be lost. An in-flight or unknown save may still have been applied by Core; leaving does not undo it.",
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
  const canSave =
    state.status === "editing" &&
    baseline.state !== "retired" &&
    !project.isFetching &&
    !project.isError &&
    (!eligibilityChanged || mode === "any" || !versions.isError) &&
    input !== null &&
    AmendEmployeeRequestSchema.safeParse(input).success;

  function current(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }
  function toggle(stage: Stage) {
    setSelected((old) =>
      old.some(
        (item) =>
          item.pipeline_version_id === stage.pipeline_version_id &&
          item.stage_id === stage.stage_id,
      )
        ? old.filter(
            (item) =>
              item.pipeline_version_id !== stage.pipeline_version_id ||
              item.stage_id !== stage.stage_id,
          )
        : [...old, stage],
    );
  }
  async function readSaved(receipt: EmployeeCommandReceipt, controller: AbortController) {
    setState({ status: "accepted", receipt, readError: false });
    try {
      const [freshProject, freshEmployee] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
        session.request(generation, (token) =>
          api.employee(projectId, employee.id, token, controller.signal),
        ),
      ]);
      if (!current(controller)) return;
      if (
        freshProject.revision < receipt.project_revision ||
        freshEmployee.revision < baseline.revision + 1
      )
        throw new Error("Stale Employee readback");
      await Promise.all([
        queries.cancelQueries({ queryKey: projectKey, exact: true }),
        queries.cancelQueries({ queryKey: profileKey, exact: true }),
      ]);
      if (!current(controller)) return;
      queries.setQueryData(projectKey, freshProject);
      queries.setQueryData(profileKey, freshEmployee);
      await queries.invalidateQueries(
        { queryKey: ["live", generation, projectId, "employees"] },
        { throwOnError: true },
      );
      if (current(controller)) onSaved();
    } catch {
      if (current(controller)) setState({ status: "accepted", receipt, readError: true });
    }
  }
  async function submit(attempt: AmendEmployeeAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        api.amendEmployee(
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
        ),
      );
      if (current(controller)) await readSaved(receipt, controller);
    } catch (error) {
      if (current(controller))
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
      const [freshProject, freshEmployee] = await Promise.all([
        project.refetch(),
        session.request(generation, (token) =>
          api.employee(projectId, employee.id, token, controller.signal),
        ),
      ]);
      if (freshProject.isError || !freshProject.data) throw freshProject.error;
      if (mode === "only") {
        const freshVersions = await versions.refetch();
        if (freshVersions.isError) throw freshVersions.error;
      }
      if (!current(controller)) return;
      queries.setQueryData(profileKey, freshEmployee);
      setBaseline(freshEmployee);
      setState({ status: "editing" });
    } catch (error) {
      if (current(controller)) setBaselineError(describeApiError(error, "Employee"));
    } finally {
      if (current(controller)) setRefreshing(false);
    }
  }
  const disabled = state.status !== "editing" || baseline.state === "retired";
  return (
    <section
      aria-label="Edit Employee"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <h3 className="font-semibold">Edit Employee</h3>
      <p className="text-sm text-muted-foreground">
        Changes affect future assignments. Lowering capacity does not stop active Runs.
      </p>
      {project.isPending && <p role="status">Loading Project revision…</p>}
      {project.isError && <p role="alert">{describeApiError(project.error)}</p>}
      {project.isError && (
        <Button variant="outline" onClick={() => void project.refetch()}>
          Retry Project read
        </Button>
      )}
      {baseline.state === "retired" && <p role="alert">Retired Employees cannot be edited.</p>}
      <form
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSave && input)
            void submit(amendEmployeeAttempt(AmendEmployeeRequestSchema.parse(input)));
        }}
      >
        <label className="block space-y-1 text-sm font-medium">
          Employee name
          <Input
            ref={nameInput}
            maxLength={200}
            disabled={disabled}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label className="block space-y-1 text-sm font-medium">
          Role
          <Input
            maxLength={128}
            disabled={disabled}
            value={role}
            onChange={(event) => setRole(event.target.value)}
          />
        </label>
        <label className="block space-y-1 text-sm font-medium">
          Capacity (concurrent Runs)
          <Input
            type="number"
            min={1}
            max={65535}
            step={1}
            disabled={disabled}
            value={capacity}
            onChange={(event) => setCapacity(event.target.value)}
          />
        </label>
        <fieldset className="space-y-2" disabled={disabled}>
          <legend className="text-sm font-medium">Stage eligibility</legend>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name="edit-eligibility"
              checked={mode === "any"}
              onChange={() => setMode("any")}
            />
            Any employee stage
          </label>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name="edit-eligibility"
              checked={mode === "only"}
              onChange={() => setMode("only")}
            />
            Only selected stages
          </label>
        </fieldset>
        {mode === "only" && (
          <div className="space-y-3 rounded-lg border border-border p-3">
            <p className="text-sm">Selected: {selected.length} of 128 stage targets</p>
            {selected.length > 0 && (
              <ul aria-label="Selected stages" className="space-y-1 text-sm">
                {selected.map((stage) => (
                  <li
                    key={`${stage.pipeline_version_id}:${stage.stage_id}`}
                    className="flex flex-wrap items-center gap-2 [overflow-wrap:anywhere]"
                  >
                    {stage.stage_id} in Pipeline version {stage.pipeline_version_id}
                    <Button
                      type="button"
                      variant="outline"
                      disabled={disabled}
                      onClick={() => toggle(stage)}
                      aria-label={`Remove ${stage.stage_id} from ${stage.pipeline_version_id}`}
                    >
                      Remove
                    </Button>
                  </li>
                ))}
              </ul>
            )}
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
                    const choice = { pipeline_version_id: version.id, stage_id: stage.id };
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
                        {version.name} v{version.version}: {stage.name}
                      </label>
                    );
                  }),
              )}
            <nav aria-label="Edit stage pages" className="flex items-center justify-between gap-2">
              <Button
                type="button"
                variant="outline"
                disabled={previous.length === 0 || versions.isFetching}
                onClick={() => {
                  setCursor(previous.at(-1) ?? null);
                  setPrevious(previous.slice(0, -1));
                }}
              >
                Previous versions
              </Button>
              <span className="text-sm">Page {previous.length + 1}</span>
              <Button
                type="button"
                variant="outline"
                disabled={!versions.data?.next_cursor || versions.isFetching}
                onClick={() => {
                  if (versions.data?.next_cursor) {
                    setPrevious([...previous, cursor]);
                    setCursor(versions.data.next_cursor);
                  }
                }}
              >
                Next versions
              </Button>
            </nav>
          </div>
        )}
        {state.status === "refused" && <p role="alert">{refusalMessage(state.error)}</p>}
        {state.status === "unknown" && (
          <p role="alert">Save outcome unknown. Retry the same request to obtain its receipt.</p>
        )}
        {state.status === "accepted" && (
          <p role="status">
            Employee change accepted. Receipt {state.receipt.command_id}.{" "}
            {state.readError
              ? "Profile readback failed; retry the read."
              : "Loading canonical profile…"}
          </p>
        )}
        {baselineError && <p role="alert">{baselineError}</p>}
        <div className="flex flex-wrap gap-2">
          {state.status === "editing" && (
            <Button type="submit" disabled={!canSave}>
              Save Employee
            </Button>
          )}
          {state.status === "sending" && (
            <Button type="button" disabled>
              Saving…
            </Button>
          )}
          {state.status === "unknown" && (
            <Button
              type="button"
              disabled={inFlight.current}
              onClick={() => void submit(state.attempt)}
            >
              Retry same save
            </Button>
          )}
          {state.status === "refused" && (
            <Button type="button" disabled={refreshing} onClick={() => void refreshBaseline()}>
              Refresh edit baselines
            </Button>
          )}
          {state.status === "accepted" && state.readError && (
            <Button
              type="button"
              onClick={() => {
                const controller = lifetime.current;
                if (controller) void readSaved(state.receipt, controller);
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
              Close edit
            </Button>
          )}
        </div>
      </form>
    </section>
  );
}
