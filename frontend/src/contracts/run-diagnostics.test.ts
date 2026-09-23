import assert from "node:assert/strict";
import { test } from "node:test";
import { RunDetailViewSchema, RunDiagnosticsSchema } from "./index.ts";
import {
  emptyRunDiagnosticsFixture,
  populatedRunDiagnosticsFixture,
  runViewFixture,
} from "./run-fixtures.ts";

test("diagnostics accept empty and safe populated metadata without changing wire values", () => {
  for (const raw of [emptyRunDiagnosticsFixture, populatedRunDiagnosticsFixture]) {
    assert.strictEqual(RunDiagnosticsSchema.parse(raw), raw);
    assert.strictEqual(
      RunDetailViewSchema.parse({ ...runViewFixture, diagnostics: raw }).diagnostics,
      raw,
    );
  }
});

test("diagnostics require every envelope field", () => {
  for (const field of Object.keys(emptyRunDiagnosticsFixture)) {
    const raw: Record<string, unknown> = { ...emptyRunDiagnosticsFixture };
    delete raw[field];
    assert.equal(RunDiagnosticsSchema.safeParse(raw).success, false, field);
  }
  for (const raw of [undefined, null, [], "diagnostics"]) {
    assert.equal(RunDiagnosticsSchema.safeParse(raw).success, false);
  }
});

test("report markers reject private bodies and false availability", () => {
  for (const field of ["runtime_report", "handoff", "proxy_usage", "git_source"]) {
    for (const value of [null, { available: true }]) {
      assert.equal(
        RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, [field]: value }).success,
        true,
        field,
      );
    }
    for (const value of [
      {},
      { available: false },
      { available: true, prompt: "private" },
      { available: true, object_key: "private" },
      { status: "finished", body: "private" },
      [],
      "private",
    ]) {
      assert.equal(
        RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, [field]: value }).success,
        false,
        field,
      );
    }
  }
});

test("incidents, evidence and streams reject content and object references", () => {
  const incident = populatedRunDiagnosticsFixture.incidents[0];
  const evidence = populatedRunDiagnosticsFixture.evidence[0];
  const stream = populatedRunDiagnosticsFixture.streams[0];
  for (const [field, item] of [
    ["incidents", { ...incident, kind: "private" }],
    ["incidents", { ...incident, assessment: "secret" }],
    ["evidence", { ...evidence, object_key: "private" }],
    ["evidence", { ...evidence, body: "private" }],
    ["evidence", { ...evidence, content_availability: "available" }],
    ["streams", { ...stream, raw: "private" }],
  ] as const) {
    assert.equal(
      RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, [field]: [item] }).success,
      false,
      field,
    );
  }
  for (const field of ["incidents", "evidence", "streams"]) {
    for (const value of [null, {}, "text", [null], [1], [[]]]) {
      assert.equal(
        RunDiagnosticsSchema.safeParse({ ...emptyRunDiagnosticsFixture, [field]: value }).success,
        false,
        field,
      );
    }
  }
});

test("diagnostics reject additive top-level fields before they enter the Run cache", () => {
  for (const extra of [
    { object_key: "private" },
    { prompt: "private" },
    JSON.parse('{"__proto__":{"private":true}}'),
  ]) {
    assert.equal(
      RunDetailViewSchema.safeParse({
        ...runViewFixture,
        diagnostics: { ...populatedRunDiagnosticsFixture, ...extra },
      }).success,
      false,
    );
  }
  assert.equal(Object.hasOwn(Object.prototype, "private"), false);
});
