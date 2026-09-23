import { useEffect, useRef, useState } from "react";
import { useQueryClient, type QueryKey } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import {
  createPropertySchemaAttempt,
  type PropertySchemaAttempt,
} from "../contracts/property-schema-command.ts";
import type { ProjectTaskPropertySchemaSchema } from "../contracts/task-property-schema.ts";
import type { z } from "zod";
import { LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendPropertySchemaCommand } from "./property-schema-command-api.ts";
import type { ProjectReadScope } from "./read-scope.ts";

type Baseline = z.infer<typeof ProjectTaskPropertySchemaSchema>;
type State =
  | { kind: "editing" | "confirm" }
  | { kind: "sending" | "unknown"; attempt: PropertySchemaAttempt }
  | { kind: "refused"; error: LiveCommandError }
  | { kind: "accepted"; revision: number; readError: boolean };

export function TaskPropertySchemaEditor({
  api,
  session,
  generation,
  projectId,
  leaveGuard,
  baseline,
  queryKey,
}: ProjectReadScope & { baseline: Baseline; queryKey: QueryKey }) {
  const queries = useQueryClient();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [original, setOriginal] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [state, setState] = useState<State>({ kind: "editing" });
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const pending = state.kind === "sending" || state.kind === "unknown";
  const dirty = open && (text !== original || pending);

  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (!dirty) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave schema editing? Unsaved fields and the retry key will be lost. Core may have applied an in-flight command.",
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
  }, [leaveGuard, dirty]);

  function begin() {
    const value = JSON.stringify(baseline.schema, null, 2);
    setText(value);
    setOriginal(value);
    setError(null);
    setState({ kind: "editing" });
    setOpen(true);
  }
  function makeAttempt() {
    return createPropertySchemaAttempt(
      projectId,
      baseline.project_revision,
      JSON.parse(text) as unknown,
    );
  }
  function review() {
    try {
      makeAttempt();
      setError(null);
      setState({ kind: "confirm" });
    } catch {
      setError(
        "Enter a complete schema with matching keys, valid definitions and tagged default values.",
      );
    }
  }
  async function submit(attempt: PropertySchemaAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendPropertySchemaCommand(
          fetch,
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (controller.signal.aborted || session.getSnapshot().generation !== generation) return;
      setState({ kind: "accepted", revision: receipt.project_revision, readError: false });
      try {
        const fresh = await session.request(generation, (token) =>
          api.taskPropertySchema(projectId, token, controller.signal),
        );
        if (fresh.project_revision < receipt.project_revision)
          throw new Error("readback behind receipt");
        await queries.cancelQueries({ queryKey, exact: true });
        if (controller.signal.aborted || session.getSnapshot().generation !== generation) return;
        queries.setQueryData(queryKey, fresh);
        setOpen(false);
      } catch {
        if (!controller.signal.aborted)
          setState({ kind: "accepted", revision: receipt.project_revision, readError: true });
      }
    } catch (caught) {
      if (!controller.signal.aborted)
        setState(
          caught instanceof LiveCommandError && caught.kind !== "outcome_unknown"
            ? { kind: "refused", error: caught }
            : { kind: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }

  if (!open)
    return (
      <Button variant="outline" onClick={begin}>
        Configure schema
      </Button>
    );
  return (
    <div className="space-y-3 rounded border border-border p-3">
      <label className="block text-sm font-medium" htmlFor="task-property-schema-json">
        Complete schema JSON
      </label>
      <p className="text-xs text-muted-foreground">
        Saving replaces all definitions. Core permits this only before the first Task exists. Each
        definition needs key, display_name, property_type, required, default_value and
        allowed_choices; use null when a default or choice set is absent.
      </p>
      <textarea
        id="task-property-schema-json"
        className="min-h-48 w-full rounded border border-input bg-background p-2 font-mono text-xs"
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          setState({ kind: "editing" });
          setError(null);
        }}
        disabled={pending || state.kind === "accepted"}
        spellCheck={false}
      />
      {error && <p role="alert">{error}</p>}
      {state.kind === "confirm" && (
        <p role="status">
          Confirm full replacement at Project revision {baseline.project_revision}.
        </p>
      )}
      {state.kind === "refused" && (
        <p role="alert">
          {state.error.kind === "stale_revision"
            ? "Project changed. Refresh the schema baseline, review your edits and confirm again."
            : state.error.kind === "conflict"
              ? "This Project already has a Task, so its property schema is locked."
              : `Core refused schema configuration (${state.error.kind}). Check the definitions or refresh the baseline.`}
        </p>
      )}
      {state.kind === "unknown" && (
        <p role="alert">Outcome unknown. Retry the exact request to obtain its receipt.</p>
      )}
      {state.kind === "accepted" && (
        <p role="status">
          Core accepted Project revision {state.revision}.
          {state.readError ? " Readback failed; refresh before another edit." : ""}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {(state.kind === "editing" || state.kind === "refused") && (
          <Button disabled={text === original} onClick={review}>
            Review save
          </Button>
        )}
        {state.kind === "confirm" && (
          <Button
            onClick={() => {
              try {
                void submit(makeAttempt());
              } catch {
                setState({ kind: "editing" });
                setError("Schema changed while confirming. Review it again.");
              }
            }}
          >
            Confirm replacement
          </Button>
        )}
        {state.kind === "unknown" && (
          <Button onClick={() => void submit(state.attempt)}>Retry exact request</Button>
        )}
        <Button
          variant="outline"
          disabled={pending}
          onClick={() => {
            if (leaveGuard.canLeave()) setOpen(false);
          }}
        >
          Close editor
        </Button>
      </div>
    </div>
  );
}
