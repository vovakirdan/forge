import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ArtifactViewSchema,
  LifecycleStatusSchema,
  ProjectViewSchema,
  TaskDetailViewSchema,
  TaskKindSchema,
  TaskListResponseSchema,
  TaskSummaryViewSchema,
  WaitConditionViewSchema,
} from "./index.ts";
import {
  ids,
  projectFixture,
  taskDetailFixture,
  taskSummaryFixture,
  timestamp,
} from "./fixtures.ts";

test("Project is a control projection, without invented catalogs", () => {
  const project = ProjectViewSchema.parse(projectFixture);
  assert.deepEqual(project, projectFixture);
  for (const missing of ["priority_scheme", "cancellation_reasons", "property_schema"]) {
    assert.equal(Object.hasOwn(project, missing), false);
  }
  assert.equal(
    ProjectViewSchema.safeParse({ ...projectFixture, execution_gate: "running" }).success,
    false,
  );
});

test("Task supports every Core lifecycle and both kinds independently of stage names", () => {
  const expectedLifecycles = ["draft", "ready", "in_progress", "waiting", "done", "cancelled"];
  const expectedKinds = ["delivery", "analysis"];
  assert.deepEqual(LifecycleStatusSchema.options, expectedLifecycles);
  assert.deepEqual(TaskKindSchema.options, expectedKinds);
  for (const lifecycle of expectedLifecycles) {
    for (const kind of expectedKinds) {
      const parsed = TaskSummaryViewSchema.parse({ ...taskSummaryFixture, lifecycle, kind });
      assert.equal(parsed.lifecycle, lifecycle);
      assert.equal(parsed.kind, kind);
    }
  }
  assert.equal(
    TaskSummaryViewSchema.safeParse({ ...taskSummaryFixture, kind: "research" }).success,
    false,
  );
  assert.equal(
    TaskSummaryViewSchema.safeParse({ ...taskSummaryFixture, lifecycle: "review" }).success,
    false,
  );
});

test("analysis without Git retains arbitrary stage, priority and every wait", () => {
  const task = TaskDetailViewSchema.parse(taskDetailFixture);
  assert.deepEqual(task, taskDetailFixture);
  assert.equal(task.current_stage_id, "choose_path");
  assert.equal(task.priority, "owner_defined_urgency");
  assert.deepEqual(
    task.wait_conditions.map((wait) => wait.kind),
    ["decision_required", "vendor_response"],
  );
  for (const missing of [
    "project_id",
    "assignee",
    "reviewer",
    "git",
    "run_active",
    "cancellation_reason",
  ]) {
    assert.equal(Object.hasOwn(task, missing), false);
  }
});

test("draft and cancelled draft preserve null stage and Definition of Done", () => {
  for (const lifecycle of ["draft", "cancelled"]) {
    const raw = {
      ...taskDetailFixture,
      lifecycle,
      current_stage_id: null,
      definition_of_done: null,
      artifacts: [],
      wait_conditions: [],
      properties: {},
    };
    assert.deepEqual(TaskDetailViewSchema.parse(raw), raw);
  }
});

test("Task rejects omitted required fields, invalid types and unsafe revisions", () => {
  for (const field of Object.keys(taskDetailFixture)) {
    const raw: Record<string, unknown> = { ...taskDetailFixture };
    delete raw[field];
    assert.equal(TaskDetailViewSchema.safeParse(raw).success, false, field);
  }
  for (const revision of [0, -1, 1.5, "7", Number.MAX_SAFE_INTEGER + 1, Infinity, NaN]) {
    assert.equal(
      TaskSummaryViewSchema.safeParse({ ...taskSummaryFixture, revision }).success,
      false,
    );
    assert.equal(ProjectViewSchema.safeParse({ ...projectFixture, revision }).success, false);
  }
  assert.equal(
    TaskSummaryViewSchema.parse({ ...taskSummaryFixture, revision: Number.MAX_SAFE_INTEGER })
      .revision,
    Number.MAX_SAFE_INTEGER,
  );
  for (const patch of [
    { current_stage_id: 42 },
    { updated_at: "yesterday" },
    { priority: null },
    { key: "" },
  ]) {
    assert.equal(
      TaskSummaryViewSchema.safeParse({ ...taskSummaryFixture, ...patch }).success,
      false,
    );
  }
  assert.equal(TaskSummaryViewSchema.parse(taskSummaryFixture).key, "TASK-001");
});

test("wait optional domain values are required nullable wire fields; actors are closed", () => {
  const wait = taskDetailFixture.wait_conditions[1];
  assert.ok(wait);
  assert.deepEqual(WaitConditionViewSchema.parse(wait), wait);
  for (const field of ["detail", "source_stage_id"]) {
    const raw: Record<string, unknown> = { ...wait };
    delete raw[field];
    assert.equal(WaitConditionViewSchema.safeParse(raw).success, false);
  }
  for (const kind of ["human", "employee", "system_manager", "core", "supervisor"]) {
    assert.equal(
      WaitConditionViewSchema.safeParse({ ...wait, created_by: { kind, id: ids.actor } }).success,
      true,
    );
  }
  assert.equal(
    WaitConditionViewSchema.safeParse({ ...wait, created_by: { kind: "admin", id: ids.actor } })
      .success,
    false,
  );
});

test("artifacts preserve arbitrary JSON without interpreting object-reference lookalikes", () => {
  const values = [
    null,
    "Reported result",
    false,
    12.5,
    ["a", 1, null, { detail: true }],
    {
      storage: "object_reference",
      object_key: "not-a-fetch-request",
      media_type: "text/plain",
      content_digest: "opaque",
    },
  ];
  for (const value of values) {
    const artifact = {
      id: ids.artifact,
      kind: "report",
      title: "Synthetic result",
      metadata: { nested: value },
      body: value,
      created_at: timestamp,
    };
    assert.deepEqual(ArtifactViewSchema.parse(artifact), artifact);
  }
  const artifact = taskDetailFixture.artifacts[0];
  assert.ok(artifact);
  for (const metadata of [null, "text", 42, true, []]) {
    assert.equal(ArtifactViewSchema.safeParse({ ...artifact, metadata }).success, false);
  }
  for (const field of ["metadata", "body"]) {
    const raw: Record<string, unknown> = { ...artifact };
    delete raw[field];
    assert.equal(ArtifactViewSchema.safeParse(raw).success, false, field);
    for (const invalid of [
      undefined,
      { value: undefined },
      [Infinity],
      { nested: { fn: () => 1 } },
    ]) {
      assert.equal(ArtifactViewSchema.safeParse({ ...artifact, [field]: invalid }).success, false);
    }
  }
});

test("additional fields survive at task, artifact, wait, actor and project boundaries", () => {
  const raw = structuredClone(taskDetailFixture);
  const extended = {
    ...raw,
    future_task: { revision: "opaque" },
    artifacts: raw.artifacts.map((artifact) => ({ ...artifact, future_artifact: 1 })),
    wait_conditions: raw.wait_conditions.map((wait) => ({
      ...wait,
      future_wait: true,
      created_by: { ...wait.created_by, label: "System" },
    })),
  };
  assert.deepEqual(TaskDetailViewSchema.parse(extended), extended);
  const project = { ...projectFixture, future_catalog: { values: ["custom"] } };
  assert.deepEqual(ProjectViewSchema.parse(project), project);
});

test("artifact JSON preserves special object keys without prototype interpretation", () => {
  const value: unknown = JSON.parse(
    '{"__proto__":{"x":1},"constructor":{"nested":{"__proto__":"literal"}},"items":[{"__proto__":null}]}',
  );
  const artifact = { ...taskDetailFixture.artifacts[0], metadata: value, body: value };
  const parsed = ArtifactViewSchema.parse(artifact);
  assert.deepEqual(parsed.body, value);
  assert.deepEqual(parsed.metadata, value);
  assert.equal(JSON.stringify(parsed.body), JSON.stringify(value));
  assert.equal(Object.hasOwn(Object.prototype, "x"), false);
  const cycle: Record<string, unknown> = {};
  cycle["self"] = cycle;
  for (const body of [cycle, new Date(), new Map(), { nested: new Set([1]) }]) {
    assert.equal(ArtifactViewSchema.safeParse({ ...artifact, body }).success, false);
  }
});

test("Task pagination retains opaque cursor and omits rather than invents final cursor", () => {
  for (const page of [
    { items: [taskSummaryFixture], next_cursor: "opaque/page+2=" },
    { items: [], future_page: true },
  ]) {
    assert.deepEqual(TaskListResponseSchema.parse(page), page);
  }
  assert.equal(Object.hasOwn(TaskListResponseSchema.parse({ items: [] }), "next_cursor"), false);
  for (const invalid of [
    {},
    { items: null },
    { items: [null] },
    { items: [], next_cursor: null },
    { items: [], next_cursor: 3 },
  ]) {
    assert.equal(TaskListResponseSchema.safeParse(invalid).success, false);
  }
});
