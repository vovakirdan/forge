// Synthetic HTTP examples, not captured provider/Core output or demo-service models.
// Shapes are taken from forge-core/src/http/views.rs and nested domain serializers.
export const ids = {
  project: "01a00000-0000-7000-8000-000000000001",
  task: "01a00000-0000-7000-8000-000000000002",
  pipeline: "01a00000-0000-7000-8000-000000000003",
  version1: "01a00000-0000-7000-8000-000000000004",
  version2: "01a00000-0000-7000-8000-000000000005",
  analysisVersion: "01a00000-0000-7000-8000-000000000006",
  actor: "01a00000-0000-7000-8000-000000000007",
  artifact: "01a00000-0000-7000-8000-000000000008",
  wait1: "01a00000-0000-7000-8000-000000000009",
  wait2: "01a00000-0000-7000-8000-000000000010",
  hook: "01a00000-0000-7000-8000-000000000011",
};
export const timestamp = "2026-09-17T10:00:00.123Z";

export const projectFixture = {
  id: ids.project,
  revision: 3,
  name: "Synthetic contract project",
  execution_gate: "stopped",
};

export const taskSummaryFixture = {
  id: ids.task,
  key: "TASK-001",
  revision: 7,
  title: "Compare migration approaches",
  kind: "analysis",
  lifecycle: "waiting",
  current_stage_id: "choose_path",
  priority: "owner_defined_urgency",
  pipeline_version_id: ids.analysisVersion,
  updated_at: timestamp,
};

export const taskDetailFixture = {
  ...taskSummaryFixture,
  description: "Compare costs and report a recommendation; no Git workspace is needed.",
  definition_of_done: "Attach the comparison and an explicit recommendation.",
  cancellation: null,
  properties: {
    contains_migrations: { type: "boolean", value: true },
    notes: { type: "text", value: "" },
    estimate: { type: "number", value: 2.5 },
    suggested_date: { type: "date", value: [2026, 260] },
    approach: { type: "enum", value: "Gradual migration" },
    labels: { type: "multi_enum", value: ["Backend", "Research"] },
    related_task: {
      type: "reference",
      value: { reference_type: "task", reference_id: ids.task },
    },
  },
  artifacts: [
    {
      id: ids.artifact,
      kind: "analysis_result",
      title: "Approach comparison",
      metadata: { author: "Synthetic employee", attempts: 2, checked: false },
      body: { recommendation: "Gradual migration", alternatives: ["Big bang"] },
      created_at: timestamp,
    },
  ],
  wait_conditions: [
    {
      id: ids.wait1,
      kind: "decision_required",
      detail: "Select an approach.",
      source_stage_id: "choose_path",
      created_by: { kind: "core", id: ids.actor },
      created_at: timestamp,
    },
    {
      id: ids.wait2,
      kind: "vendor_response",
      detail: null,
      source_stage_id: null,
      created_by: { kind: "system_manager", id: ids.actor },
      created_at: timestamp,
    },
  ],
};

export const deliveryPipelineFixture = {
  id: ids.version1,
  pipeline_id: ids.pipeline,
  version: 1,
  name: "Synthetic delivery",
  catalog_revision: 4,
  default_version_id: ids.version2,
  latest_version: 2,
  deleted_at: timestamp,
  task_kinds: ["delivery"],
  entry_stage_id: "work",
  max_stage_visits: 20,
  // Deliberately not traversal order: the graph includes a rework cycle.
  stages: [
    {
      id: "integrate",
      name: "Integration",
      executor_kind: "system",
      outcomes: ["applied", "no_changes", "stale_base"],
      instructions: "",
      workspace: null,
      acceptance_policy: null,
      system_action: {
        kind: "git_integration",
        outcomes: { applied: "applied", no_changes: "no_changes", stale_base: "stale_base" },
        required_review_stages: ["review"],
      },
    },
    {
      id: "review",
      name: "Review",
      executor_kind: "employee",
      outcomes: ["approved", "rework", "unable"],
      instructions: "Inspect the exact accepted candidate.",
      workspace: { kind: "git", access: "read_only" },
      acceptance_policy: {
        kind: "candidate_review",
        independent: true,
        verdicts: { approved: "accepted", rework: "rejected", unable: "inconclusive" },
      },
      system_action: null,
    },
    {
      id: "work",
      name: "Work",
      executor_kind: "employee",
      outcomes: ["completed", "obsolete"],
      instructions: "Deliver the requested change.",
      workspace: { kind: "git", access: "read_write" },
      acceptance_policy: null,
      system_action: null,
    },
  ],
  transitions: [
    {
      from_stage_id: "work",
      outcome: "completed",
      target: { kind: "stage", stage_id: "review" },
      artifact_requirements: [{ kind: "stage_evidence", minimum_count: 1, scope: "current_stage" }],
    },
    { from_stage_id: "work", outcome: "obsolete", target: { kind: "cancelled" } },
    {
      from_stage_id: "review",
      outcome: "approved",
      target: { kind: "stage", stage_id: "integrate" },
    },
    { from_stage_id: "review", outcome: "rework", target: { kind: "stage", stage_id: "work" } },
    { from_stage_id: "review", outcome: "unable", target: { kind: "stage", stage_id: "work" } },
    { from_stage_id: "integrate", outcome: "applied", target: { kind: "done" } },
    { from_stage_id: "integrate", outcome: "no_changes", target: { kind: "done" } },
    {
      from_stage_id: "integrate",
      outcome: "stale_base",
      target: { kind: "stage", stage_id: "work" },
    },
  ],
};

export const analysisPipelineFixture = {
  id: ids.analysisVersion,
  pipeline_id: "01a00000-0000-7000-8000-000000000012",
  version: 1,
  name: "Synthetic analysis without review or QA",
  catalog_revision: 1,
  default_version_id: ids.analysisVersion,
  latest_version: 1,
  deleted_at: null,
  task_kinds: ["analysis"],
  entry_stage_id: "explore",
  max_stage_visits: 10,
  stages: [
    {
      id: "choose_path",
      name: "Choose a migration approach",
      executor_kind: "human",
      outcomes: ["selected", "more_research", "obsolete"],
      instructions: "Choose from the researched approaches.",
      workspace: null,
      acceptance_policy: null,
      system_action: null,
    },
    {
      id: "explore",
      name: "Compare approaches",
      executor_kind: "employee",
      outcomes: ["compared"],
      instructions: "Report the comparison as an artifact.",
      workspace: { kind: "filesystem", access: "read_write" },
      acceptance_policy: null,
      system_action: null,
    },
  ],
  transitions: [
    {
      from_stage_id: "explore",
      outcome: "compared",
      target: { kind: "stage", stage_id: "choose_path" },
    },
    {
      from_stage_id: "choose_path",
      outcome: "selected",
      target: { kind: "done" },
      artifact_requirements: [{ kind: "analysis_result", minimum_count: 1, scope: "task_history" }],
    },
    {
      from_stage_id: "choose_path",
      outcome: "more_research",
      target: { kind: "stage", stage_id: "explore" },
    },
    { from_stage_id: "choose_path", outcome: "obsolete", target: { kind: "cancelled" } },
  ],
};
