import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import {
  configureEmployeeRuntimeAttempt,
  type EmployeeRuntimeAttempt,
} from "../contracts/employee-runtime.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendConfigureEmployeeRuntime } from "./employee-runtime-api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { Field } from "./Field.tsx";

type State =
  | { kind: "editing" }
  | { kind: "sending" | "unknown"; attempt: EmployeeRuntimeAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; attempt: EmployeeRuntimeAttempt; readError: boolean };

export function EmployeeRuntimePanel({
  scope,
  employeeId,
}: {
  scope: ProjectReadScope;
  employeeId: string;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [open, setOpen] = useState(false);
  const [bindingText, setBindingText] = useState("");
  const [state, setState] = useState<State>({ kind: "editing" });
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const key = useMemo(
    () => ["live", generation, projectId, "employee-runtime-baseline", employeeId] as const,
    [generation, projectId, employeeId],
  );
  const metadataKey = useMemo(
    () => readKeys.employeeRuntimeMetadata(generation, projectId, employeeId),
    [generation, projectId, employeeId],
  );
  const metadata = useQuery({
    queryKey: metadataKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employeeRuntimeMetadata(projectId, employeeId, token, signal),
      ),
    retry: false,
  });
  const baseline = useQuery({
    queryKey: key,
    queryFn: async ({ signal }) => {
      const [project, employee, operations] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, signal)),
        session.request(generation, (token) => api.employee(projectId, employeeId, token, signal)),
        session.request(generation, (token) =>
          api.employeeOperations(projectId, employeeId, token, signal),
        ),
      ]);
      return { project, employee, operations };
    },
    enabled: open,
    retry: false,
  });
  useReadLifetime(key);
  useReadLifetime(metadataKey);
  useEffect(() => {
    const next = new AbortController();
    controller.current = next;
    return () => next.abort();
  }, []);
  const dirty =
    open && (bindingText.length > 0 || state.kind === "sending" || state.kind === "unknown");
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave runtime configuration? The draft and retry key will be lost. An in-flight command may still have been applied.",
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
  async function readAccepted(attempt: EmployeeRuntimeAttempt) {
    const next = controller.current;
    if (!next) return;
    setState({ kind: "accepted", attempt, readError: false });
    try {
      const [project, operations] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, next.signal)),
        session.request(generation, (token) =>
          api.employeeOperations(projectId, employeeId, token, next.signal),
        ),
      ]);
      if (
        project.revision < attempt.expectedProjectRevision + 1 ||
        !operations.runtime_binding_configured
      )
        throw new Error("runtime readback absent");
      if (!current(next)) return;
      queries.setQueryData(["project", generation, projectId], project);
      void queries.invalidateQueries({ queryKey: key });
      void queries.invalidateQueries({ queryKey: metadataKey });
      void queries.invalidateQueries({
        queryKey: readKeys.employeeOperations(generation, projectId, employeeId),
      });
      setBindingText("");
      setOpen(false);
      setConfirm(false);
      setState({ kind: "editing" });
    } catch {
      if (current(next)) setState({ kind: "accepted", attempt, readError: true });
    }
  }
  async function send(attempt: EmployeeRuntimeAttempt) {
    const next = controller.current;
    if (!next || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt });
    try {
      await session.request(generation, (token) =>
        sendConfigureEmployeeRuntime(
          fetch,
          attempt,
          token,
          AbortSignal.any([next.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(next)) await readAccepted(attempt);
    } catch (cause) {
      if (current(next))
        setState(
          cause instanceof LiveCommandError && cause.kind !== "outcome_unknown"
            ? { kind: "refused", error: cause }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  function submit() {
    if (!baseline.data || baseline.isFetching) return;
    try {
      const attempt = configureEmployeeRuntimeAttempt(
        projectId,
        employeeId,
        baseline.data.project.revision,
        JSON.parse(bindingText) as unknown,
      );
      setError(null);
      setConfirm(false);
      void send(attempt);
    } catch {
      setError(
        "Enter a valid secret-free RuntimeBinding JSON for this Project. The current form supports surface mode none.",
      );
    }
  }
  return (
    <section
      aria-label="Employee runtime configuration"
      className="space-y-3 border-t border-border pt-4"
    >
      <h3 className="font-medium">Runtime configuration</h3>
      <p className="text-xs text-muted-foreground">
        Saved profile metadata excludes prompts, credential material and host paths. Use enrolled
        credential IDs and a pinned image; never enter secret values. Changes affect future Runs
        only.
      </p>
      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          disabled={metadata.isFetching}
          onClick={() => void metadata.refetch()}
        >
          Refresh runtime metadata
        </Button>
      </div>
      {metadata.isPending && <p role="status">Loading runtime metadata…</p>}
      {metadata.isError && <p role="alert">{describeApiError(metadata.error, "Employee")}</p>}
      {metadata.data?.availability === "unavailable" && <p>No runtime binding configured.</p>}
      {metadata.data?.availability === "available" && (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 break-all text-sm">
          <Field label="Binding revision">{metadata.data.binding_revision}</Field>
          <Field label="Profile">
            {metadata.data.profile_id} (revision {metadata.data.profile_revision})
          </Field>
          <Field label="Adapter">
            {metadata.data.adapter_id} {metadata.data.adapter_version}
          </Field>
          <Field label="Provider and model">
            {metadata.data.provider_id} · {metadata.data.model}
          </Field>
          <Field label="Credential binding">{metadata.data.credential_binding_id}</Field>
          <Field label="Credential delivery">{metadata.data.credential_delivery}</Field>
          <Field label="Image digest">{metadata.data.image_digest}</Field>
          <Field label="Surface">{metadata.data.surface_kind}</Field>
          <Field label="Access">{metadata.data.access}</Field>
          <Field label="CPU limit">{metadata.data.limits.cpu_millis} milli-CPU</Field>
          <Field label="Memory limit">{metadata.data.limits.memory_bytes} bytes</Field>
          <Field label="Process limit">{metadata.data.limits.pids}</Field>
          <Field label="Run time limit">{metadata.data.limits.wall_seconds} seconds</Field>
          <Field label="Output limit">{metadata.data.budget.max_output_bytes} bytes</Field>
          <Field label="Spend limit">{metadata.data.budget.max_spend_microusd ?? "Unknown"}</Field>
          <Field label="Updated">{metadata.data.updated_at}</Field>
        </dl>
      )}
      <Button
        variant="outline"
        onClick={() => {
          if (!open || leaveGuard.canLeave()) setOpen(!open);
        }}
      >
        {open ? "Hide runtime form" : "Configure runtime"}
      </Button>
      {open && (
        <div className="space-y-3 rounded border border-border p-4">
          {baseline.isPending && <p role="status">Loading Employee and Project revision…</p>}
          {baseline.isError && <p role="alert">{describeApiError(baseline.error, "Employee")}</p>}
          {baseline.data && (
            <p className="text-sm">
              Current binding:{" "}
              {baseline.data.operations.runtime_binding_configured
                ? "Configured"
                : "Not configured"}
            </p>
          )}
          <label className="block text-sm">
            Runtime binding JSON
            <textarea
              rows={14}
              className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-xs"
              value={bindingText}
              disabled={
                state.kind === "sending" || state.kind === "unknown" || state.kind === "accepted"
              }
              onChange={(event) => setBindingText(event.target.value)}
            />
          </label>
          <p className="text-xs text-muted-foreground">
            This form accepts a complete binding with surface mode none. Use a new profile revision
            when changing an existing profile.
          </p>
          {error && <p role="alert">{error}</p>}
          {(state.kind === "editing" || state.kind === "refused") && !confirm && (
            <Button
              disabled={!baseline.data || baseline.isFetching || !bindingText.trim()}
              onClick={() => setConfirm(true)}
            >
              Review runtime configuration
            </Button>
          )}
          {confirm && (
            <div
              role="alertdialog"
              aria-label="Confirm runtime configuration"
              className="space-y-2 rounded border border-border p-3"
            >
              <p>Save this binding for future Employee Runs?</p>
              <Button onClick={submit}>Confirm</Button>
              <Button variant="outline" onClick={() => setConfirm(false)}>
                Cancel
              </Button>
            </div>
          )}
          {state.kind === "sending" && <p role="status">Saving runtime configuration…</p>}
          {state.kind === "unknown" && (
            <div className="space-y-2">
              <p role="alert">Outcome unknown. Retry the exact same request and key.</p>
              <Button onClick={() => void send(state.attempt)}>Retry same request</Button>
            </div>
          )}
          {state.kind === "refused" && (
            <div className="space-y-2">
              <p role="alert">
                Core refused runtime configuration ({state.error.kind}). Keep the draft and refresh
                the baseline.
              </p>
              <Button onClick={() => void baseline.refetch()}>Refresh baseline</Button>
            </div>
          )}
          {state.kind === "accepted" && (
            <div className="space-y-2">
              <p role="status">
                Runtime configuration accepted.{" "}
                {state.readError
                  ? "Canonical configured flag readback failed."
                  : "Reading configured flag…"}
              </p>
              {state.readError && (
                <Button onClick={() => void readAccepted(state.attempt)}>Retry status read</Button>
              )}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
