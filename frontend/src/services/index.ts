/**
 * Service layer.
 *
 * Every screen reads through these services — never from the mock module
 * directly. Swapping the in-memory repository for a REST/WebSocket backend
 * only requires re-implementing the bodies below (all methods are async).
 */
import * as db from "@/data/mockDb";
import type {
  ActivityEvent,
  Agent,
  Artifact,
  ChatChannel,
  ChatMessage,
  Comment,
  Epic,
  Finding,
  Goal,
  Pipeline,
  Project,
  Run,
  StageId,
  Task,
} from "@/data/types";

const latency = <T>(value: T, ms = 90): Promise<T> =>
  new Promise((resolve) => setTimeout(() => resolve(structuredClone(value)), ms));

/* ------------------------------------------------------------------ */

export const ProjectService = {
  list: () => latency(db.projects),
  get: (id: string) => latency(db.projects.find((p) => p.id === id) ?? db.projects[0]),
  current: () => latency(db.projects[0]),
  integrations: () => latency(db.integrations),
  risks: () => latency(db.risks),
};

export const AgentService = {
  list: () => latency(db.agents),
  get: (id: string) => latency(db.agents.find((a) => a.id === id) ?? null),
  reports: (id: string) => latency(db.agents.filter((a) => a.managerId === id)),
  roles: () =>
    latency([
      "Lead",
      "Compiler Developer",
      "Backend Developer",
      "Reviewer",
      "QA",
      "DevOps",
      "Researcher",
      "Custom Role",
    ]),
  engines: () =>
    latency([
      { id: "codex", name: "Codex CLI", detail: "Local binary · high throughput" },
      { id: "claude", name: "Claude Code", detail: "Local binary · strong review quality" },
      { id: "opencode", name: "OpenCode", detail: "Not detected on this host" },
      { id: "custom", name: "Custom", detail: "Bring your own executor command" },
    ]),
  capabilities: () =>
    latency([
      "implement",
      "research",
      "review",
      "test",
      "create findings",
      "propose tasks",
      "architecture decisions",
      "delegate",
      "merge",
      "deploy",
    ]),
  hire: (draft: Partial<Agent>) => latency({ ...draft, id: `agt-${Date.now()}` } as Agent, 250),
};

export const GoalService = {
  list: () => latency(db.goals),
  get: (id: string) => latency(db.goals.find((g) => g.id === id) ?? null),
  epics: (goalId?: string) =>
    latency(goalId ? db.epics.filter((e) => e.goalId === goalId) : db.epics),
  epic: (id: string) => latency(db.epics.find((e) => e.id === id) ?? null),
  planNextWave: (epicId: string) => latency({ epicId, created: 3 }, 400),
};

export const TaskService = {
  list: () => latency(db.tasks),
  get: (id: string) => latency(db.tasks.find((t) => t.id === id) ?? null),
  byStage: (stage: StageId) => latency(db.tasks.filter((t) => t.stage === stage)),
  comments: (taskId: string) => latency(db.comments.filter((c) => c.taskId === taskId)),
  artifacts: (taskId: string) => latency(db.artifacts.filter((a) => a.taskId === taskId)),
  findings: (taskId?: string) =>
    latency(taskId ? db.findings.filter((f) => f.taskId === taskId) : db.findings),
  promoteFinding: (findingId: string) => latency({ findingId, taskId: "TASK-153" }, 300),
  updateFindingStatus: (findingId: string, status: Finding["status"]) =>
    latency({ findingId, status }, 200),
  move: (taskId: string, stage: StageId) => latency({ taskId, stage }, 150),
};

export const PipelineService = {
  list: () => latency(db.pipelines),
  get: (id: string) => latency(db.pipelines.find((p) => p.id === id) ?? db.pipelines[0]),
  availableStages: () => latency(db.availableStages),
};

export const ChatService = {
  channels: () => latency(db.channels),
  messages: (channelId: string) => latency(db.messages.filter((m) => m.channelId === channelId)),
  send: (channelId: string, body: string) =>
    latency<ChatMessage>(
      {
        id: `m-${Date.now()}`,
        channelId,
        authorId: "human",
        body,
        createdAt: new Date().toISOString(),
      },
      120,
    ),
  /** Mocked Lead reply — in production this is a structured status push. */
  leadReply: (channelId: string, prompt: string) =>
    latency<ChatMessage>(
      {
        id: `m-${Date.now() + 1}`,
        channelId,
        authorId: "agt-max",
        body: mockLeadAnswer(prompt),
        createdAt: new Date().toISOString(),
      },
      900,
    ),
};

function mockLeadAnswer(prompt: string) {
  const p = prompt.toLowerCase();
  if (p.includes("status") || p.includes("happening") || p.includes("now"))
    return "Three runs are active: Bob on TASK-142 implementation attempt #3, verification on TASK-143, and Alice reviewing TASK-144. Two tasks are waiting — TASK-146 and TASK-152 — both behind the drop-flag ABI decision.";
  if (p.includes("block") || p.includes("waiting"))
    return "TASK-152 needs your approval on the ABI representation. TASK-146 inherits that block. Nothing else is stalled; the verification queue is long but draining.";
  if (p.includes("risk"))
    return "Main risk is the heavy verification pool: capacity 1 with 4 queued, roughly 48 minutes of drain. Secondary risk is TASK-142 sitting at attempt 3 of 5.";
  if (p.includes("plan") || p.includes("wave"))
    return "Wave 3 has 5 executable tasks. I will not expand the backlog until it closes — future items stay as high-level ideas under each epic.";
  return "Noted. I will fold that into the current wave without creating new executable tasks — findings stay findings until you or I promote them.";
}

export const RunService = {
  list: () => latency(db.runs),
  get: (id: string) => latency(db.runs.find((r) => r.id === id) ?? null),
  active: () => latency(db.runs.filter((r) => r.status === "running")),
  forTask: (taskId: string) => latency(db.runs.filter((r) => r.taskId === taskId)),
};

export const ActivityService = {
  list: (category?: ActivityEvent["category"]) =>
    latency(category ? db.activity.filter((e) => e.category === category) : db.activity),
};

export const KnowledgeService = {
  policies: () => latency(db.policies),
  decisions: () => latency(db.decisions),
  lessons: () => latency(db.lessons),
  skillPacks: () => latency(db.skillPacks),
  codeIndexes: () => latency(db.codeIndexes),
};

export const ResourceService = {
  host: () => latency(db.host),
  pools: () => latency(db.resourcePools),
};

export type {
  ActivityEvent,
  Agent,
  Artifact,
  ChatChannel,
  ChatMessage,
  Comment,
  Epic,
  Finding,
  Goal,
  Pipeline,
  Project,
  Run,
  Task,
};
