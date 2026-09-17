import type { RunDiagnostics as RunDiagnosticsView } from "../contracts/run-diagnostics.ts";
import { presentRunDiagnostics } from "../presentation/run-diagnostics.ts";
import { Field } from "./Field.tsx";

export function RunDiagnostics({ diagnostics }: { diagnostics: RunDiagnosticsView }) {
  const { presentation } = presentRunDiagnostics(diagnostics);
  return (
    <section aria-label="Diagnostics" className="space-y-3">
      <h3 className="font-medium">Diagnostics</h3>
      <p className="text-xs text-muted-foreground">
        Availability and loaded counts do not establish success or acceptance. Diagnostic bodies
        arrive in this response but are not displayed; linked objects are not fetched automatically.
      </p>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <Field label="Runtime report">
          {presentation.runtimeReportAvailable ? "Available" : "Unavailable"}
        </Field>
        <Field label="Handoff">{presentation.handoffAvailable ? "Available" : "Unavailable"}</Field>
        <Field label="Proxy usage">
          {presentation.proxyUsageAvailable ? "Available" : "Unavailable"}
        </Field>
        <Field label="Git source">
          {presentation.gitSourceAvailable ? "Available" : "Unavailable"}
        </Field>
        <Field label="Loaded incidents">{presentation.loadedIncidentCount}</Field>
        <Field label="Loaded evidence">{presentation.loadedEvidenceCount}</Field>
        <Field label="Loaded incomplete streams">{presentation.loadedIncompleteStreamCount}</Field>
      </dl>
      {diagnostics.streams.length === 0 ? (
        <p className="text-sm">No streams loaded.</p>
      ) : (
        <ul aria-label="Loaded streams" className="space-y-2">
          {diagnostics.streams.map((stream, index) => (
            <li key={index} className="rounded border border-border p-3">
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
                <Field label="Stream">{stream.stream}</Field>
                <Field label="Incomplete">{stream.incomplete ? "Yes" : "No"}</Field>
              </dl>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
