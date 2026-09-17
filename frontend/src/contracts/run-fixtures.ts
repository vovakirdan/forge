import { ids } from "./fixtures.ts";

// Synthetic wire examples based on Core views and assignment serializers.
// These do not represent provider execution, accepted evidence or live diagnostics.
export const runIds = {
  run: "01a00000-0000-7000-8000-000000000020",
  employee: "01a00000-0000-7000-8000-000000000021",
  assignment: "01a00000-0000-7000-8000-000000000022",
  queueEntry: "01a00000-0000-7000-8000-000000000023",
  thread: "01a00000-0000-7000-8000-000000000024",
  message: "01a00000-0000-7000-8000-000000000025",
  escalation: "01a00000-0000-7000-8000-000000000026",
  invocation: "01a00000-0000-7000-8000-000000000027",
  candidate: "01a00000-0000-7000-8000-000000000028",
  job: "01a00000-0000-7000-8000-000000000029",
  jobAttempt: "01a00000-0000-7000-8000-000000000030",
};

export const assignmentFixtures = [
  {
    purpose: "task_stage",
    owner: { task_id: ids.task, queue_entry_id: runIds.queueEntry, stage_id: "work" },
  },
  {
    purpose: "communication",
    owner: {
      assignment_id: runIds.assignment,
      thread_id: runIds.thread,
      source_message_id: runIds.message,
    },
  },
  {
    purpose: "resolution",
    owner: {
      assignment_id: runIds.assignment,
      escalation_id: runIds.escalation,
      lease_generation: 1,
    },
  },
  {
    purpose: "hook",
    owner: {
      invocation_id: runIds.invocation,
      task_id: ids.task,
      pipeline_version_id: ids.version1,
      stage_id: "verify_delivery",
      stage_visit: 2,
      candidate_proposal_id: runIds.candidate,
    },
  },
  {
    purpose: "system_job",
    owner: {
      job_id: runIds.job,
      attempt_id: runIds.jobAttempt,
      generation: 1,
      kind: "summarization",
    },
  },
  {
    purpose: "system_job",
    owner: { job_id: runIds.job, attempt_id: runIds.jobAttempt, generation: 2, kind: "onboarding" },
  },
];

export const runViewFixture = {
  id: runIds.run,
  task_id: ids.task,
  assignment: assignmentFixtures[0],
  employee_id: runIds.employee,
  stage_id: "work",
  attempt: 1,
  desired_state: "stop_requested",
  observed_state: "running",
  lease_fencing_token: 15,
  environment_epoch: 1,
  last_observed_sequence: 0,
  run_spec_version: 6,
};

export const emptyRunDiagnosticsFixture = {
  runtime_report: null,
  handoff: null,
  incidents: [],
  evidence: [],
  streams: [],
  proxy_usage: null,
  git_source: null,
};

export const populatedRunDiagnosticsFixture = {
  runtime_report: { status: "finished", provider_detail: { text: "Synthetic unverified report" } },
  handoff: { summary: "Partial synthetic work", checkpoint: null },
  incidents: [{ kind: "provider_unavailable", assessment: null, annotations: ["synthetic"] }],
  evidence: [
    { storage: "object_reference", object_key: "opaque/not-a-fetch-request", accepted: false },
  ],
  streams: [
    { stream: "runtime.custom", incomplete: true },
    { stream: "stdout", incomplete: false },
  ],
  proxy_usage: { input_tokens: 20, output_tokens: null },
  git_source: { mode: "synthetic_snapshot", candidate: { label: "opaque" } },
};
