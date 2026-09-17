import assert from "node:assert/strict";
import { test } from "node:test";
import { RunDiagnosticsSchema } from "../contracts/index.ts";
import {
  emptyRunDiagnosticsFixture,
  populatedRunDiagnosticsFixture,
} from "../contracts/run-fixtures.ts";
import { presentRunDiagnostics } from "./index.ts";
import { freezeFixture } from "./test-helpers.ts";

test("Unavailable reports and empty loaded arrays do not imply success or zero usage", () => {
  const source = RunDiagnosticsSchema.parse(emptyRunDiagnosticsFixture);
  const result = presentRunDiagnostics(source);
  assert.strictEqual(result.source, source);
  assert.deepEqual(result.presentation, {
    runtimeReportAvailable: false,
    handoffAvailable: false,
    proxyUsageAvailable: false,
    gitSourceAvailable: false,
    loadedIncidentCount: 0,
    loadedEvidenceCount: 0,
    loadedIncompleteStreamCount: 0,
  });
});

test("Report availability distinguishes null from an empty object independently", () => {
  const reports = [
    ["runtime_report", "runtimeReportAvailable"],
    ["handoff", "handoffAvailable"],
    ["proxy_usage", "proxyUsageAvailable"],
    ["git_source", "gitSourceAvailable"],
  ] as const;
  for (const [field, availability] of reports) {
    const source = RunDiagnosticsSchema.parse({ ...emptyRunDiagnosticsFixture, [field]: {} });
    const result = presentRunDiagnostics(source);
    for (const [, key] of reports) {
      assert.equal(result.presentation[key], key === availability);
    }
    assert.strictEqual(result.source[field], source[field]);
  }
});

test("Diagnostics count loaded rows and incomplete streams, not acceptance or total history", () => {
  const source = RunDiagnosticsSchema.parse(populatedRunDiagnosticsFixture);
  const result = presentRunDiagnostics(source);
  assert.deepEqual(result.presentation, {
    runtimeReportAvailable: true,
    handoffAvailable: true,
    proxyUsageAvailable: true,
    gitSourceAvailable: true,
    loadedIncidentCount: 1,
    loadedEvidenceCount: 1,
    loadedIncompleteStreamCount: 1,
  });
  assert.strictEqual(result.source.streams, source.streams);
  assert.strictEqual(result.source.evidence, source.evidence);
});

test("Unknown and duplicate stream names stay loaded facts without deduplication", () => {
  const source = RunDiagnosticsSchema.parse({
    ...emptyRunDiagnosticsFixture,
    streams: [
      { stream: "owner.custom", incomplete: true },
      { stream: "owner.custom", incomplete: true },
      { stream: "stderr", incomplete: false },
    ],
  });
  const result = presentRunDiagnostics(source);
  assert.equal(result.presentation.loadedIncompleteStreamCount, 2);
  assert.strictEqual(result.source.streams, source.streams);
});

test("Diagnostics preserve arbitrary JSON without parsing it, fetching it or modifying it", (context) => {
  const fetch = context.mock.method(globalThis, "fetch", () => {
    throw new Error("Presentation must not fetch reports or evidence");
  });
  const opaque = JSON.parse(
    '{"__proto__":{"retained":true},"constructor":{"opaque":true},"accepted":true,"url":"https://invalid.example/not-a-fetch-request"}',
  );
  const source = freezeFixture(
    RunDiagnosticsSchema.parse({
      ...emptyRunDiagnosticsFixture,
      runtime_report: opaque,
      evidence: [opaque],
      extra: opaque,
    }),
  );
  const before = JSON.stringify(source);
  const result = presentRunDiagnostics(source);
  assert.strictEqual(result.source, source);
  assert.strictEqual(result.source.runtime_report, opaque);
  assert.strictEqual(result.source.evidence[0], opaque);
  assert.strictEqual(result.source["extra"], opaque);
  assert.deepEqual(result, presentRunDiagnostics(source));
  assert.equal(JSON.stringify(source), before);
  assert.equal(fetch.mock.callCount(), 0);
});
