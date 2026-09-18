import assert from "node:assert/strict";
import { test } from "node:test";
import { PriorityLevelViewSchema, PrioritySchemeViewSchema } from "./priority-scheme.ts";

const projectId = "01a08a41-977c-7d23-a347-22c7afcfb653";
const level = { id: "owner_defined", display_name: "По возможности", rank: -4, retired: false };
const scheme = {
  project_id: projectId,
  project_revision: 9,
  default_level_id: level.id,
  levels: [level],
};

test("priority schemes preserve 1, 3 or 10 arbitrary levels without rank ordering or a fixed enum", () => {
  for (const count of [1, 3, 10]) {
    const levels = Array.from({ length: count }, (_, index) => ({
      ...level,
      id: `owner_${index}`,
      display_name: `Важность ${index}`,
      rank: index % 2 === 0 ? -1 : -2,
    }));
    const wire = { ...scheme, levels, default_level_id: "owner_0" };
    assert.strictEqual(PrioritySchemeViewSchema.parse(wire), wire);
    assert.deepEqual(
      wire.levels.map((entry) => entry.id),
      levels.map((entry) => entry.id),
    );
  }
  for (const rank of [-2_147_483_648, 2_147_483_647]) {
    assert.equal(PriorityLevelViewSchema.parse({ ...level, rank }).rank, rank);
  }
});

test("scheme invariants reject duplicates, empty levels and absent or retired defaults", () => {
  for (const patch of [
    { levels: [] },
    { levels: [level, { ...level, rank: 42 }] },
    { default_level_id: "not_defined" },
    { levels: [{ ...level, retired: true }] },
  ]) {
    assert.equal(PrioritySchemeViewSchema.safeParse({ ...scheme, ...patch }).success, false);
  }
  const wire = { ...scheme, levels: [level, { ...level, id: "historical", retired: true }] };
  assert.strictEqual(PrioritySchemeViewSchema.parse(wire), wire);
});

test("priority fields are strictly typed and bounded; reads do not coerce or inject defaults", () => {
  for (const patch of [
    { id: "UPPER" },
    { id: "../other" },
    { id: "x".repeat(65) },
    { rank: "1" },
    { rank: 1.5 },
    { rank: -2_147_483_649 },
    { rank: 2_147_483_648 },
    { retired: "false" },
    { retired: undefined },
    { display_name: null },
  ]) {
    assert.equal(PriorityLevelViewSchema.safeParse({ ...level, ...patch }).success, false);
  }
  for (const patch of [
    { project_id: "not-an-id" },
    { project_revision: 0 },
    { project_revision: "1" },
    { project_revision: Number.MAX_SAFE_INTEGER + 1 },
    { default_level_id: null },
    { levels: null },
  ]) {
    assert.equal(PrioritySchemeViewSchema.safeParse({ ...scheme, ...patch }).success, false);
  }
});

test("priority labels match Rust Unicode scalar and whitespace bounds without trimming", () => {
  for (const display_name of ["", " \n\t", "\u0085", "😀".repeat(129), "\ud800", "\udfff"]) {
    assert.equal(PriorityLevelViewSchema.safeParse({ ...level, display_name }).success, false);
  }
  for (const display_name of ["😀".repeat(128), "\ufeff", "  Custom label  ", "<img src=x>"]) {
    assert.equal(
      PriorityLevelViewSchema.parse({ ...level, display_name }).display_name,
      display_name,
    );
  }
});

test("additive JSON fields remain unmodified, including nested __proto__ data", () => {
  const wire: unknown = JSON.parse(
    JSON.stringify(scheme).replace(
      '"levels":[{',
      '"extra":{"__proto__":{"source":"data"}},"levels":[{"__proto__":{"also":"data"},',
    ),
  );
  const parsed = PrioritySchemeViewSchema.parse(wire);
  assert.strictEqual(parsed, wire);
  assert.equal(Object.hasOwn(parsed.levels[0] ?? {}, "__proto__"), true);
  assert.deepEqual(parsed["extra"], JSON.parse('{"__proto__":{"source":"data"}}'));
});
