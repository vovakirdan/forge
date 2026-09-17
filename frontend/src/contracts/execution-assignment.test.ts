import assert from "node:assert/strict";
import { test } from "node:test";
import { ExecutionAssignmentSchema, SystemJobKindSchema } from "./index.ts";
import { assignmentFixtures } from "./run-fixtures.ts";

test("ExecutionAssignment accepts all five explicit purposes and both SystemJob kinds", () => {
  assert.deepEqual(
    assignmentFixtures.map((fixture) => fixture.purpose),
    ["task_stage", "communication", "resolution", "hook", "system_job", "system_job"],
  );
  assert.deepEqual(SystemJobKindSchema.options, ["summarization", "onboarding"]);
  for (const fixture of assignmentFixtures) {
    assert.deepEqual(ExecutionAssignmentSchema.parse(fixture), fixture);
  }
});

test("ExecutionAssignment rejects unknown purpose, missing owner and every missing owner field", () => {
  for (const invalid of [{}, { purpose: "task" }, { purpose: "background", owner: {} }]) {
    assert.equal(ExecutionAssignmentSchema.safeParse(invalid).success, false);
  }
  for (const fixture of assignmentFixtures) {
    for (const owner of [undefined, null, [], "owner", {}]) {
      assert.equal(ExecutionAssignmentSchema.safeParse({ ...fixture, owner }).success, false);
    }
    for (const field of Object.keys(fixture.owner)) {
      const owner: Record<string, unknown> = { ...fixture.owner };
      delete owner[field];
      assert.equal(
        ExecutionAssignmentSchema.safeParse({ ...fixture, owner }).success,
        false,
        `${fixture.purpose}.${field}`,
      );
    }
  }
});

test("assignment identities, keys and closed SystemJob kinds are checked without coercion", () => {
  for (const fixture of assignmentFixtures) {
    for (const [field, value] of Object.entries(fixture.owner)) {
      if (field.endsWith("_id") && field !== "stage_id") {
        for (const invalid of [null, 1, "not-a-uuid", String(value).replace("-7000-", "-4000-")]) {
          assert.equal(
            ExecutionAssignmentSchema.safeParse({
              ...fixture,
              owner: { ...fixture.owner, [field]: invalid },
            }).success,
            false,
            field,
          );
        }
      }
      if (field === "stage_id") {
        assert.equal(
          ExecutionAssignmentSchema.safeParse({
            ...fixture,
            owner: { ...fixture.owner, stage_id: "Invalid stage" },
          }).success,
          false,
        );
      }
    }
    if (fixture.purpose === "system_job") {
      assert.equal(
        ExecutionAssignmentSchema.safeParse({
          ...fixture,
          owner: { ...fixture.owner, kind: "cleanup" },
        }).success,
        false,
      );
    }
  }
});

test("generations and stage visits are positive safe integers", () => {
  for (const fixture of assignmentFixtures) {
    for (const field of ["lease_generation", "stage_visit", "generation"]) {
      if (!Object.hasOwn(fixture.owner, field)) continue;
      for (const value of [1, Number.MAX_SAFE_INTEGER]) {
        const raw = { ...fixture, owner: { ...fixture.owner, [field]: value } };
        assert.deepEqual(ExecutionAssignmentSchema.parse(raw), raw);
      }
      for (const value of [0, -1, 1.5, "1", null, Number.MAX_SAFE_INTEGER + 1, Infinity, NaN]) {
        assert.equal(
          ExecutionAssignmentSchema.safeParse({
            ...fixture,
            owner: { ...fixture.owner, [field]: value },
          }).success,
          false,
          field,
        );
      }
    }
  }
});

test("assignment parsing preserves additive JSON keys on every purpose and owner without mutation", () => {
  const extra: Record<string, unknown> = JSON.parse(
    '{"__proto__":{"literal":true},"constructor":{"__proto__":null}}',
  );
  for (const fixture of assignmentFixtures) {
    const raw = { ...fixture, ...extra, owner: { ...fixture.owner, ...extra } };
    const before = structuredClone(raw);
    Object.freeze(raw.owner);
    Object.freeze(raw);
    assert.deepEqual(ExecutionAssignmentSchema.parse(raw), before);
    assert.deepEqual(raw, before);
  }
});
