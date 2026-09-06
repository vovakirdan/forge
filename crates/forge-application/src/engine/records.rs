//! Infrastructure-neutral snapshots used by the shared command transaction.
use forge_domain::{
    Artifact, ArtifactProducer, CommandId, Employee, EventId, PipelineId, PipelineVersionId,
    ProjectId, StageId, Task, TaskId, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

/// Scheduler-only fields stored alongside a typed Task snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TaskPersistence {
    /// Priority rank copied from the owning Project scheme for indexed ordering.
    pub priority_rank: i32,
    /// Completed or attempted stage executions retained for scheduler projections.
    pub attempt_count: u32,
    /// Reserved future work-surface identity, absent in M0.
    pub task_work_surface_id: Option<Uuid>,
}

/// A typed Task with its queryable scheduler projection.
#[derive(Clone, Debug)]
pub struct StoredTask {
    /// Canonical domain state.
    pub task: Task,
    /// Indexed scheduler rank.
    pub persistence: TaskPersistence,
}

/// A typed Employee snapshot.
#[derive(Clone, Debug)]
pub struct StoredEmployee {
    /// Canonical domain state.
    pub employee: Employee,
}

/// Context in which immutable Artifact evidence was attached.
#[derive(Clone, Debug)]
pub struct ArtifactLocation {
    /// Owning Task when evidence is Task-local.
    pub task_id: Option<TaskId>,
    /// Producing Run when evidence came from an executor.
    pub run_id: Option<Uuid>,
    /// Pipeline stage that accepted the evidence.
    pub stage_id: Option<StageId>,
    /// Scope that produced the evidence.
    pub producer: ArtifactProducer,
    /// Optional producer identity, such as the Employee ID.
    pub producer_id: Option<Uuid>,
    /// Structured, non-authoritative producer metadata.
    pub producer_data: Value,
}

impl ArtifactLocation {
    /// Creates the standard direct-human attachment context.
    #[must_use]
    pub fn human(task_id: Option<TaskId>) -> Self {
        Self {
            task_id,
            run_id: None,
            stage_id: None,
            producer: ArtifactProducer::Human,
            producer_id: None,
            producer_data: json!({}),
        }
    }
}

/// Immutable Artifact plus its durable attachment context.
#[derive(Clone, Debug)]
pub struct StoredArtifact {
    /// Canonical immutable Artifact state.
    pub artifact: Artifact,
    /// Durable Task/Run/stage attachment data.
    pub location: ArtifactLocation,
}

/// Durable result cached for an idempotent named command.
#[derive(Clone, Debug)]
pub struct IdempotencyRecord {
    /// Project command scope.
    pub project_id: ProjectId,
    /// Caller-provided idempotency key.
    pub key: String,
    /// Stable logical command identity.
    pub command_id: CommandId,
    /// Named command path value.
    pub command_name: String,
    /// Optimistic Project revision presented by the caller.
    pub expected_revision: Option<u64>,
    /// Canonical request hash used to reject mismatched key reuse.
    pub request_hash: String,
    /// Safe actor object.
    pub actor: Value,
    /// Full stable response receipt.
    pub receipt: Value,
    /// Event created by the command when one exists.
    pub event_id: Option<EventId>,
    /// Project revision visible in the receipt.
    pub response_revision: Option<u64>,
}

/// Exact storage vocabulary for a queue-entry lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    /// Ready for an eligible Employee and Project execution gate.
    Queued,
    /// Claimed inside a scheduler transaction before Lease creation.
    Leased,
    /// Cancelled before execution.
    Cancelled,
    /// Finished with a terminal queue outcome.
    Completed,
    /// Retained after an unrecoverable scheduler failure.
    DeadLetter,
}

/// Input for a deduplicated queued Employee stage.
#[derive(Clone, Debug)]
pub struct QueueEntryInput {
    /// Owning Project.
    pub project_id: ProjectId,
    /// Task to dispatch.
    pub task_id: TaskId,
    /// Stable Task ordering sequence.
    pub task_sequence: u64,
    /// Pinned Pipeline identity.
    pub pipeline_id: PipelineId,
    /// Pinned immutable Pipeline version.
    pub pipeline_version_id: PipelineVersionId,
    /// Employee-executed stage identity.
    pub stage_id: StageId,
    /// Task revision represented by this entry.
    pub task_revision: u64,
    /// Positive attempt ordinal for this Task stage.
    pub attempt_number: u32,
    /// Project priority key copied from the Task.
    pub priority_level_id: String,
    /// Indexed priority rank copied from the Project scheme.
    pub priority_rank: i32,
    /// Non-authoritative resource request object.
    pub resource_profile: Value,
    /// First time at which this entry may be dispatched.
    pub eligible_at: Timestamp,
}

/// Durable scheduler queue projection.
#[derive(Clone, Debug)]
pub struct QueueEntry {
    /// Queue identity.
    pub id: Uuid,
    /// Input identity and ordering fields.
    pub input: QueueEntryInput,
    /// Current durable queue state.
    pub state: QueueState,
}

/// Only the fenced identities needed by canonical stop commands; no RunSpec or secrets.
#[derive(Clone, Debug)]
pub struct ActiveRun {
    /// Durable Run identity.
    pub id: Uuid,
    /// Owning Project scope.
    pub project_id: ProjectId,
    /// Canonical Task currently served by this Run.
    pub task_id: TaskId,
    /// Lease generation required by every stop-side compare-and-set.
    pub lease_fencing_token: u64,
    /// Runtime environment generation paired with the Lease fence.
    pub environment_epoch: u64,
}
