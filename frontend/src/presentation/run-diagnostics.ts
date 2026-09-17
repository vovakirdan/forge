import type { RunDiagnostics } from "../contracts/run-diagnostics.ts";

/** Availability and loaded counts are not acceptance, success or history totals. */
export function presentRunDiagnostics<TDiagnostics extends RunDiagnostics>(
  diagnostics: TDiagnostics,
) {
  return {
    source: diagnostics,
    presentation: {
      runtimeReportAvailable: diagnostics.runtime_report !== null,
      handoffAvailable: diagnostics.handoff !== null,
      proxyUsageAvailable: diagnostics.proxy_usage !== null,
      gitSourceAvailable: diagnostics.git_source !== null,
      loadedIncidentCount: diagnostics.incidents.length,
      loadedEvidenceCount: diagnostics.evidence.length,
      loadedIncompleteStreamCount: diagnostics.streams.filter((stream) => stream.incomplete).length,
    },
  };
}

export type PresentedRunDiagnostics = ReturnType<typeof presentRunDiagnostics>;
