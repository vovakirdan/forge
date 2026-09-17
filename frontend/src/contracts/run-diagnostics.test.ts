import assert from "node:assert/strict";
import { test } from "node:test";
import { RunDetailViewSchema, RunDiagnosticsSchema } from "./index.ts";
import {
  emptyRunDiagnosticsFixture,
  populatedRunDiagnosticsFixture,
  runViewFixture,
} from "./run-fixtures.ts";

test("diagnostics accepts absent measurements as null, partial reports, and populated reports", () => {
  const partial = {
    ...emptyRunDiagnosticsFixture,
    runtime_report: { status: "started" },
    streams: [{ stream: "custom_stream", incomplete: true }],
  };
  for (const raw of [emptyRunDiagnosticsFixture, partial, populatedRunDiagnosticsFixture]) {
    assert.deepEqual(RunDiagnosticsSchema.parse(raw), raw);
  }
  const diagnostics = RunDiagnosticsSchema.parse(emptyRunDiagnosticsFixture);
  assert.equal(diagnostics.proxy_usage, null);
  assert.equal(diagnostics.git_source, null);
  assert.equal(Object.hasOwn(diagnostics, "cost"), false);
  assert.equal(Object.hasOwn(diagnostics, "accepted"), false);
});

test("diagnostics envelope fields are all required even when empty or null", () => {
  for (const field of Object.keys(emptyRunDiagnosticsFixture)) {
    const raw: Record<string, unknown> = { ...emptyRunDiagnosticsFixture };
    delete raw[field];
    assert.equal(RunDiagnosticsSchema.safeParse(raw).success, false, field);
  }
  for (const raw of [undefined, null, [], "diagnostics"]) {
    assert.equal(RunDiagnosticsSchema.safeParse(raw).success, false);
  }
});

test("diagnostic report roots are nullable JSON objects, not primitives or arrays", () => {
  for (const field of ["runtime_report", "handoff", "proxy_usage", "git_source"]) {
    for (const value of [null, {}, { nested: [1, true, null, "text", { more: 2.5 }] }]) {
      const raw = { ...emptyRunDiagnosticsFixture, [field]: value };
      assert.deepEqual(RunDiagnosticsSchema.parse(raw), raw);
    }
    for (const value of [
      undefined,
      true,
      12,
      "text",
      [],
      { not_json: undefined },
      { not_json: Infinity },
    ]) {
      assert.equal(
        RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, [field]: value }).success,
        false,
        field,
      );
    }
  }
});

test("incidents and evidence are arrays of opaque JSON objects without automatic interpretation", () => {
  for (const field of ["incidents", "evidence"]) {
    const item = {
      storage: "object_reference",
      object_key: "opaque/no-fetch",
      nested: [null, false],
      unknown: "synthetic",
    };
    const raw = { ...emptyRunDiagnosticsFixture, [field]: [item] };
    assert.deepEqual(RunDiagnosticsSchema.parse(raw), raw);
    for (const value of [null, {}, "text", [null], [1], [[]], [{ invalid: NaN }]]) {
      assert.equal(
        RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, [field]: value }).success,
        false,
        field,
      );
    }
  }
});

test("stream entries require an open string name and explicit incomplete boolean", () => {
  const stream = {
    stream: "provider.specific/stream",
    incomplete: true,
    future: { hint: "opaque" },
  };
  assert.deepEqual(
    RunDiagnosticsSchema.parse({ ...emptyRunDiagnosticsFixture, streams: [stream] }).streams,
    [stream],
  );
  for (const streams of [
    null,
    {},
    [null],
    [{}],
    [{ stream: "stdout" }],
    [{ incomplete: false }],
    [{ stream: 1, incomplete: false }],
    [{ stream: "stdout", incomplete: "false" }],
  ]) {
    assert.equal(
      RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, streams }).success,
      false,
    );
  }
});

test("diagnostic JSON and additive keys retain special members without fetching, merging or mutation", () => {
  const special: Record<string, unknown> = JSON.parse(
    '{"__proto__":{"literal":true},"constructor":{"__proto__":null},"nested":[{"__proto__":"data"}]}',
  );
  const diagnostics = {
    runtime_report: special,
    handoff: special,
    incidents: [special],
    evidence: [special],
    streams: [{ stream: "stdout", incomplete: false, ...special }],
    proxy_usage: special,
    git_source: special,
    ...special,
  };
  const raw = { ...runViewFixture, diagnostics, ...special };
  const before = structuredClone(raw);
  Object.freeze(special);
  Object.freeze(diagnostics);
  Object.freeze(raw);
  const parsed = RunDetailViewSchema.parse(raw);
  assert.deepEqual(parsed, before);
  assert.deepEqual(raw, before);
  assert.equal(JSON.stringify(parsed), JSON.stringify(before));
  assert.equal(Object.hasOwn(Object.prototype, "literal"), false);
});
