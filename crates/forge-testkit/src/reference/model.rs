//! Inspectable persistence snapshots for command conformance, not a runtime simulator.

use std::collections::{BTreeMap, BTreeSet};

use forge_application::ports::{
    ActiveRun, IdempotencyRecord, QueueEntryInput, StoredArtifact, StoredTask,
};
use forge_domain::{
    Actor, ArtifactId, DomainEvent, Employee, EmployeeId, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, TaskDependency, TaskId,
};
use serde_json::Value;
use uuid::Uuid;

/// Queue states needed by command-side invalidation; no scheduler lives here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryQueueState {
    /// Eligible intent, not yet leased.
    Queued,
    /// Fixture representing a scheduler-issued Lease.
    Leased,
    /// Retained invalidated or stopped intent.
    Cancelled,
}

/// Retained command-side queue intent.
#[derive(Clone, Debug)]
pub struct MemoryQueueEntry {
    /// Generated queue identity.
    pub id: Uuid,
    /// Canonical enqueue input.
    pub input: QueueEntryInput,
    /// Current command-side state.
    pub state: MemoryQueueState,
}

/// Active runtime fixture used only to exercise fenced command mutations.
#[derive(Clone, Debug)]
pub struct MemoryRun {
    /// Scope exposed through the application port.
    pub scope: ActiveRun,
    /// Queue entry leased by this fixture.
    pub queue_entry_id: Uuid,
    /// Commands must retain this reservation until terminal observation.
    pub lease_active: bool,
    /// Physical reservation is distinct from logical Task state.
    pub reservation_held: bool,
    /// None means running; Some(false) graceful, Some(true) forced stop.
    pub stop_requested: Option<bool>,
}

/// Immutable event with its transactionally assigned per-project sequence.
#[derive(Clone, Debug)]
pub struct MemoryEvent {
    /// Original canonical event, including actor and authoritative time.
    pub event: DomainEvent,
    /// Monotonically increasing sequence within its project.
    pub project_sequence: u64,
}

/// Pending outbox item; delivery is intentionally outside this reference backend.
#[derive(Clone, Debug)]
pub struct MemoryOutbox {
    /// Independent delivery record identity.
    pub id: Uuid,
    /// Owning project.
    pub project_id: ProjectId,
    /// Immutable source event identity.
    pub event_id: EventId,
    /// Versioned delivery subject.
    pub subject: String,
    /// Exact envelope retained at commit.
    pub envelope: Value,
}

/// An isolated copy suitable for exact before/after rollback assertions.
#[derive(Clone, Debug, Default)]
pub struct MemorySnapshot {
    pub command_handoffs: BTreeMap<forge_domain::CommandId, forge_domain::TaskHandoff>,
    pub resolver_routes: BTreeMap<(ProjectId, String), forge_domain::resolution::ResolverRoute>,
    pub escalations: BTreeMap<Uuid, forge_domain::resolution::Escalation>,
    pub resolution_assignments: BTreeMap<Uuid, forge_domain::resolution::ResolutionAssignment>,
    pub findings: BTreeMap<Uuid, forge_domain::finding::Finding>,
    /// Retained one-shot Employee assignment instructions and their terminal results.
    pub dispatch_constraints: BTreeMap<Uuid, forge_domain::NextRunEmployeeConstraint>,
    pub resume_schedules: BTreeMap<Uuid, forge_domain::TaskResumeSchedule>,
    /// Explicit operator waivers, separate from Employee receipts.
    pub message_waivers: BTreeMap<Uuid, forge_domain::communication::MessageRequirementWaiver>,
    /// Immutable operator-selected Project source allowlist.
    pub project_repositories: BTreeMap<Uuid, forge_domain::ProjectRepository>,
    /// Durable task-optional Employee conversations.
    pub employee_threads: BTreeMap<Uuid, forge_domain::communication::EmployeeThread>,
    /// Immutable messages; delivery is separate from this command reference.
    pub employee_messages: BTreeMap<Uuid, forge_domain::communication::EmployeeMessage>,
    /// Canonical projects.
    pub projects: BTreeMap<ProjectId, Project>,
    /// Canonical tasks and scheduling projections.
    pub tasks: BTreeMap<TaskId, StoredTask>,
    /// Canonical employees.
    pub employees: BTreeMap<EmployeeId, Employee>,
    /// Pipeline catalog.
    pub pipelines: BTreeMap<PipelineId, Pipeline>,
    /// Immutable pipeline versions.
    pub pipeline_versions: BTreeMap<PipelineVersionId, PipelineVersion>,
    /// Immutable artifacts and their attachment context.
    pub artifacts: BTreeMap<ArtifactId, StoredArtifact>,
    /// One canonical relation for both dependency directions and its author.
    pub dependencies: BTreeMap<(ProjectId, TaskId, TaskId), (TaskDependency, Actor)>,
    /// Retained queue intents, including cancelled revisions.
    pub queue: Vec<MemoryQueueEntry>,
    /// Active runtime fixtures; the reference engine cannot create a Run.
    pub runs: Vec<MemoryRun>,
    /// Projects whose recovery dispatch hold is set.
    pub recovery_holds: BTreeSet<ProjectId>,
    /// Ordered immutable event log.
    pub events: Vec<MemoryEvent>,
    /// Pending delivery intents.
    pub outbox: Vec<MemoryOutbox>,
    /// Original receipts keyed by project and caller idempotency key.
    pub idempotency: BTreeMap<(ProjectId, String), IdempotencyRecord>,
}
