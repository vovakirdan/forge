export type StageId =
  | "planned"
  | "ready"
  | "implementation"
  | "verification"
  | "review"
  | "integration"
  | "waiting"
  | "done";

export type Priority = "critical" | "high" | "medium" | "low";

export type AgentState = "active" | "working" | "idle" | "suspended" | "blocked";

export type EngineId = "codex" | "claude" | "opencode" | "custom";

export interface Project {
  id: string;
  key: string;
  name: string;
  repository: string;
  defaultBranch: string;
  environment: "local" | "staging" | "production";
  environmentStatus: "healthy" | "degraded" | "offline";
  description: string;
}

export interface Agent {
  id: string;
  name: string;
  displayName: string;
  role: string;
  engine: EngineId;
  state: AgentState;
  managerId: string | null;
  currentTaskId?: string | null;
  currentRunId?: string | null;
  specialization: string;
  skills: string[];
  responsibilities: string[];
  restrictions: string[];
  familiarity: { area: string; value: number }[];
  recentTaskIds: string[];
  lessons: string[];
  usagePct: number;
  costTodayUsd: number;
  concurrency: number;
  resourceClass: string;
  worktreeRequired: boolean;
  scopePaths: string[];
  joinedAt: string;
}

export interface Goal {
  id: string;
  projectId: string;
  title: string;
  description: string;
  progress: number;
  status: "active" | "planned" | "done";
  epicIds: string[];
}

export interface Epic {
  id: string;
  goalId: string;
  title: string;
  area: string;
  progress: number;
  waveTaskIds: string[];
  futureIdeas: string[];
}

export interface DodItem {
  id: string;
  label: string;
  done: boolean;
  automated: boolean;
}

export interface TimelineEntry {
  id: string;
  stage: StageId;
  attempt: number;
  actorId: string | null;
  actorLabel: string;
  status: "completed" | "failed" | "in_progress" | "changes_requested" | "passed" | "queued";
  startedAt: string;
  durationMin: number;
  summary: string;
  details?: string[];
  runId?: string;
}

export interface Finding {
  id: string;
  taskId: string;
  title: string;
  description: string;
  severity: "low" | "medium" | "high";
  foundById: string;
  createdAt: string;
  status: "open" | "promoted" | "ignored" | "attached";
  promotedTaskId?: string;
}

export interface Artifact {
  id: string;
  taskId: string;
  kind: "commit" | "patch" | "test_report" | "review_report" | "research" | "adr" | "log";
  title: string;
  meta: string;
  createdAt: string;
}

export interface Comment {
  id: string;
  taskId: string;
  authorId: string;
  body: string;
  createdAt: string;
  kind: "review" | "note";
}

export interface Task {
  id: string;
  projectId: string;
  title: string;
  description: string;
  goalId: string;
  epicId: string;
  parentTheme: string;
  stage: StageId;
  assigneeId: string | null;
  reviewerId: string | null;
  priority: Priority;
  pipelineId: string;
  attempts: number;
  baseSha: string;
  workspace: string;
  area: string;
  createdAt: string;
  updatedAt: string;
  elapsedMin: number;
  blocked: boolean;
  blockedReason?: string;
  verification: "passed" | "failed" | "running" | "pending" | "queued";
  review: "approved" | "changes_requested" | "pending" | "in_review";
  findingIds: string[];
  commentCount: number;
  currentRunId?: string | null;
  dod: DodItem[];
  timeline: TimelineEntry[];
}

export interface PipelineStageConfig {
  id: string;
  stage: StageId;
  name: string;
  executorKind: "role" | "system" | "human";
  executor: string;
  maxAttempts?: number;
  workspace?: string;
  profile?: string;
  checks?: string[];
  resourceClass?: string;
  concurrency?: number;
  failureTransition?: StageId | null;
  actions?: string[];
  rules?: string[];
}

export interface Pipeline {
  id: string;
  name: string;
  description: string;
  appliesTo: string;
  stages: PipelineStageConfig[];
  tasksUsing: number;
}

export type ChatCardKind =
  "decision" | "review_request" | "task_completed" | "verification_failed" | "risk" | "finding";

export interface ChatCard {
  kind: ChatCardKind;
  taskId?: string;
  title: string;
  body: string;
  options?: { id: string; label: string; detail: string }[];
  actions?: string[];
}

export interface ChatMessage {
  id: string;
  channelId: string;
  authorId: string;
  body: string;
  createdAt: string;
  card?: ChatCard;
}

export interface ChatChannel {
  id: string;
  name: string;
  kind: "agent" | "project" | "system";
  participantId?: string;
  subtitle: string;
  unread: number;
}

export interface RunCommand {
  id: string;
  at: string;
  tool: string;
  command: string;
  status: "ok" | "failed" | "running";
  durationSec: number;
}

export interface Run {
  id: string;
  taskId: string;
  agentId: string;
  engine: EngineId;
  reason: string;
  status: "running" | "completed" | "failed" | "queued";
  startedAt: string;
  durationSec: number;
  resourceClass: string;
  workspace: string;
  context: string[];
  tools: string[];
  commands: RunCommand[];
  filesChanged: { path: string; added: number; removed: number }[];
  output: string[];
  tokensIn: number;
  tokensOut: number;
  cpuPct: number;
  ramGb: number;
}

export interface ActivityEvent {
  id: string;
  at: string;
  category: "tasks" | "runs" | "agents" | "reviews" | "verification" | "decisions" | "system";
  actorId: string | null;
  title: string;
  detail?: string;
  taskId?: string;
  runId?: string;
}

export interface Policy {
  id: string;
  title: string;
  body: string;
  authority: "project" | "org" | "team";
  status: "active" | "draft" | "retired";
}

export interface Decision {
  id: string;
  title: string;
  body: string;
  status: "accepted" | "proposed" | "superseded";
  supersedes?: string;
  decidedAt: string;
}

export interface Lesson {
  id: string;
  body: string;
  sources: string[];
  confidence: number;
  extractedAt: string;
}

export interface SkillPack {
  id: string;
  name: string;
  description: string;
  usedBy: string[];
  version: string;
}

export interface CodeIndex {
  id: string;
  repo: string;
  symbols: number;
  indexedAt: string;
  providers: { name: string; connected: boolean }[];
}

export interface ResourcePool {
  id: string;
  name: string;
  capacity: number;
  inUse: number;
  priority: "high" | "normal" | "background";
  cpuWeight: number;
  memoryBudgetGb: number;
  ioWeight: number;
  exclusive: boolean;
}

export interface HostResources {
  cpuThreadsUsed: number;
  cpuThreadsTotal: number;
  cpuPct: number;
  ramUsedGb: number;
  ramTotalGb: number;
  diskIo: "low" | "moderate" | "high";
  activeRuns: number;
  maxRuns: number;
  heavyInUse: number;
  heavyCapacity: number;
  queues: { name: string; waiting: number }[];
}

export interface Integration {
  id: string;
  name: string;
  category: string;
  status: "connected" | "not_configured" | "error";
  detail: string;
}
