import assert from "node:assert/strict";
import { test } from "node:test";
import { TaskPropertiesSchema, TaskPropertyValueSchema } from "./index.ts";
import { ids, taskDetailFixture } from "./fixtures.ts";

test("all tagged property types retain their wire representation", () => {
  assert.deepEqual(
    TaskPropertiesSchema.parse(taskDetailFixture.properties),
    taskDetailFixture.properties,
  );
  assert.deepEqual(TaskPropertiesSchema.parse({}), {});
  assert.deepEqual(TaskPropertyValueSchema.parse({ type: "multi_enum", value: [] }), {
    type: "multi_enum",
    value: [],
  });
});

test("date is exactly a numeric year/ordinal tuple, not a string or timestamp", () => {
  for (const value of [
    [2024, 366],
    [2026, 1],
    [2026, 365],
  ]) {
    assert.deepEqual(TaskPropertyValueSchema.parse({ type: "date", value }), {
      type: "date",
      value,
    });
  }
  for (const value of [
    "2026-09-17",
    [2026],
    [2026, 260, 0],
    ["2026", 260],
    [2026, 0],
    [2026, 367],
    [2026, 1.5],
  ]) {
    assert.equal(TaskPropertyValueSchema.safeParse({ type: "date", value }).success, false);
  }
});

test("typed references support every aggregate and preserve additional fields", () => {
  for (const reference_type of ["project", "task", "artifact", "employee"]) {
    const raw = {
      type: "reference",
      value: { reference_type, reference_id: ids.task, label: "Future label" },
      future_property: true,
    };
    assert.deepEqual(TaskPropertyValueSchema.parse(raw), raw);
  }
  for (const value of [
    ids.task,
    { reference_type: "run", reference_id: ids.task },
    { reference_type: "task" },
    { reference_type: "task", reference_id: 42 },
  ]) {
    assert.equal(TaskPropertyValueSchema.safeParse({ type: "reference", value }).success, false);
  }
});

test("properties reject untagged, unknown, missing and wrongly typed values", () => {
  for (const raw of [
    "plain text",
    null,
    { type: "unknown", value: "x" },
    { type: "text" },
    { type: "boolean", value: "true" },
    { type: "number", value: "2" },
    { type: "number", value: NaN },
    { type: "number", value: Infinity },
    { type: "text", value: false },
    { type: "enum", value: [] },
    { type: "multi_enum", value: [1] },
  ]) {
    assert.equal(TaskPropertyValueSchema.safeParse(raw).success, false);
  }
  assert.equal(
    TaskPropertiesSchema.safeParse({ "not a stable key": { type: "text", value: "x" } }).success,
    false,
  );
});
