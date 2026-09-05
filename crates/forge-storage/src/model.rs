//! Storage-facing projections and JSON conversion helpers.

use forge_domain::{
    AggregateRef, AggregateType, Artifact, ArtifactBody, ArtifactProducer, CommandId, DomainEvent,
    Employee, EmployeeId, EventId, Pipeline, PipelineId, PipelineVersion, PipelineVersionId,
    Project, ProjectId, StageId, Task, TaskId, Timestamp,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::StorageError;

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

/// Immutable event data reconstructed from the append-only event log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredEvent {
    /// Global immutable event identity.
    pub id: EventId,
    /// Owning Project.
    pub project_id: ProjectId,
    /// Monotonic sequence scoped to the Project.
    pub project_sequence: u64,
    /// Aggregate category.
    pub aggregate_type: AggregateType,
    /// Aggregate identity as a UUID because the log spans aggregate types.
    pub aggregate_id: Uuid,
    /// Aggregate revision after the change.
    pub aggregate_revision: u64,
    /// Stable event vocabulary key.
    pub event_type: String,
    /// Event envelope version.
    pub schema_version: u16,
    /// Actor object retained without widening its authority.
    pub actor: Value,
    /// Optional command identity; canonical Core writes always provide one.
    pub command_id: Option<CommandId>,
    /// Optional human-readable audit reason.
    pub reason: Option<String>,
    /// Event payload object.
    pub payload: Value,
    /// Authoritative event time.
    pub occurred_at: Timestamp,
}

impl StoredEvent {
    /// Builds an append-ready event with a sequence assigned under a Project lock.
    pub fn from_domain(event: &DomainEvent, project_sequence: u64) -> Result<Self, StorageError> {
        Ok(Self {
            id: event.id(),
            project_id: event.project_id(),
            project_sequence,
            aggregate_type: event.aggregate().aggregate_type(),
            aggregate_id: aggregate_uuid(event.aggregate()),
            aggregate_revision: event.aggregate_revision(),
            event_type: enum_text(&event.kind(), "event.type")?,
            schema_version: event.schema_version(),
            actor: value_object(&event.actor(), "event.actor")?,
            command_id: Some(event.command_id()),
            reason: event.reason().map(ToOwned::to_owned),
            payload: event.payload().clone(),
            occurred_at: event.occurred_at(),
        })
    }

    /// Returns the durable v1 NATS subject derived only from safe identifiers.
    #[must_use]
    pub fn subject(&self) -> String {
        format!(
            "forge.v1.project.{}.event.{}",
            self.project_id, self.event_type
        )
    }

    /// Returns the versioned JSON envelope persisted in the transactional outbox.
    #[must_use]
    pub fn envelope(&self) -> Value {
        json!({
            "schema_version": self.schema_version,
            "event_id": self.id,
            "project_id": self.project_id,
            "project_sequence": self.project_sequence,
            "aggregate_type": self.aggregate_type,
            "aggregate_id": self.aggregate_id,
            "aggregate_revision": self.aggregate_revision,
            "event_type": self.event_type,
            "actor": self.actor,
            "command_id": self.command_id,
            "reason": self.reason,
            "payload": self.payload,
            "occurred_at": self.occurred_at,
        })
    }
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

/// Exact schema vocabulary for desired Run state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunDesiredState {
    /// Core has committed a provisioning request.
    ProvisionRequested,
    /// Core expects work to be running.
    Running,
    /// Core requested a graceful stop.
    StopRequested,
    /// Core requested forced termination.
    ForceStopRequested,
    /// Core recorded a final stopped result.
    Stopped,
    /// Core recorded a final failure.
    Failed,
}

/// Exact schema vocabulary for Supervisor-observed Run state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunObservedState {
    /// No Supervisor observation arrived yet.
    Unknown,
    /// Supervisor is provisioning the isolated environment.
    Provisioning,
    /// Supervisor observed the executor working.
    Running,
    /// Supervisor is stopping the executor.
    Stopping,
    /// Supervisor observed a clean stop.
    Stopped,
    /// Supervisor observed an execution failure.
    Failed,
    /// Supervisor can no longer observe the Run.
    Lost,
}

/// Read projection for one durable Run.
#[derive(Clone, Debug)]
pub struct RunProjection {
    /// Run identity.
    pub id: Uuid,
    /// Owning Project and Task identities.
    pub project_id: ProjectId,
    /// Task whose stage is being executed.
    pub task_id: TaskId,
    /// Queue and Lease identities that fenced this Run.
    pub queue_entry_id: Uuid,
    /// Lease identity.
    pub lease_id: Uuid,
    /// Assigned Employee.
    pub employee_id: EmployeeId,
    /// Pipeline stage identity.
    pub stage_id: String,
    /// Stage attempt ordinal.
    pub attempt_number: u32,
    /// Fencing token copied from the Lease.
    pub lease_fencing_token: u64,
    /// Environment epoch accepted by the Run.
    pub environment_epoch: u64,
    /// Highest observed or accepted submission sequence.
    pub last_sequence: u64,
    /// Core's desired lifecycle.
    pub desired_state: RunDesiredState,
    /// Supervisor's latest observed lifecycle.
    pub observed_state: RunObservedState,
    /// Positive schema version of the immutable Run specification.
    pub run_spec_version: u16,
    /// Opaque immutable Run specification.
    pub run_spec: Value,
    /// Immutable context inputs captured before provisioning the Run.
    pub context_manifest: Value,
    /// Non-authoritative Supervisor details object.
    pub observed_details: Value,
}

/// Domain revalidation required before a decoded canonical snapshot is exposed.
pub(crate) trait SnapshotValidatable {
    /// Checks aggregate-local invariants without performing cross-aggregate I/O.
    fn validate_storage_snapshot(&self) -> Result<(), forge_domain::DomainError>;
}

macro_rules! snapshot_validatable {
    ($($aggregate:ty),+ $(,)?) => {
        $(
            impl SnapshotValidatable for $aggregate {
                fn validate_storage_snapshot(&self) -> Result<(), forge_domain::DomainError> {
                    self.validate_snapshot()
                }
            }
        )+
    };
}

snapshot_validatable!(Project, Task, Pipeline, PipelineVersion, Employee, Artifact);

pub(crate) fn encode_value(value: &Value, aggregate: &'static str) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|source| StorageError::Snapshot { aggregate, source })
}

pub(crate) fn value_object<T: Serialize>(
    value: &T,
    field: &'static str,
) -> Result<Value, StorageError> {
    let value = serde_json::to_value(value).map_err(|source| StorageError::Snapshot {
        aggregate: field,
        source,
    })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(StorageError::InvalidJsonShape {
            field,
            expected: "object",
        })
    }
}

pub(crate) fn encode_object<T: Serialize>(
    value: &T,
    field: &'static str,
) -> Result<String, StorageError> {
    encode_value(&value_object(value, field)?, field)
}

pub(crate) fn encode_snapshot<T: Serialize>(
    value: &T,
    aggregate: &'static str,
) -> Result<String, StorageError> {
    encode_object(value, aggregate)
}

pub(crate) fn decode_snapshot<T: DeserializeOwned + SnapshotValidatable>(
    value: &str,
    aggregate: &'static str,
) -> Result<T, StorageError> {
    let snapshot: T = serde_json::from_str(value)
        .map_err(|source| StorageError::Snapshot { aggregate, source })?;
    snapshot
        .validate_storage_snapshot()
        .map_err(|source| StorageError::SnapshotInvariant { aggregate, source })?;
    Ok(snapshot)
}

pub(crate) fn enum_text<T: Serialize>(
    value: &T,
    field: &'static str,
) -> Result<String, StorageError> {
    match serde_json::to_value(value).map_err(|source| StorageError::Snapshot {
        aggregate: field,
        source,
    })? {
        Value::String(value) => Ok(value),
        _ => Err(StorageError::InvalidInput {
            reason: format!("{field} must serialize as a string enum"),
        }),
    }
}

pub(crate) fn u64_to_i64(value: u64, field: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

pub(crate) fn i64_to_u64(value: i64, field: &'static str) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

pub(crate) fn u32_to_i32(value: u32, field: &'static str) -> Result<i32, StorageError> {
    i32::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

pub(crate) fn i32_to_u32(value: i32, field: &'static str) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

pub(crate) fn database_timestamp(value: Timestamp) -> OffsetDateTime {
    value.as_offset_date_time()
}

pub(crate) fn domain_timestamp(value: OffsetDateTime) -> Timestamp {
    Timestamp::from_offset_date_time(value)
}

pub(crate) fn aggregate_uuid(value: AggregateRef) -> Uuid {
    match value {
        AggregateRef::Project(id) => id.as_uuid(),
        AggregateRef::Task(id) => id.as_uuid(),
        AggregateRef::Employee(id) => id.as_uuid(),
        AggregateRef::Artifact(id) => id.as_uuid(),
        AggregateRef::Pipeline(id) => id.as_uuid(),
        AggregateRef::PipelineVersion(id) => id.as_uuid(),
    }
}

pub(crate) fn artifact_body_columns(
    artifact: &Artifact,
) -> Result<(String, Option<String>, Option<String>), StorageError> {
    match artifact.body() {
        ArtifactBody::InlineJson { value } => Ok((
            "inline_json".to_owned(),
            Some(encode_value(value, "artifact.body")?),
            None,
        )),
        ArtifactBody::ObjectReference {
            object_key,
            media_type,
            content_digest,
        } => Ok((
            "object_reference".to_owned(),
            None,
            Some(encode_value(
                &json!({
                    "object_key": object_key,
                    "media_type": media_type,
                    "content_digest": content_digest,
                }),
                "artifact.object_reference",
            )?),
        )),
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Project, ProjectId, Timestamp};
    use serde_json::json;

    use super::{StorageError, decode_snapshot};

    #[test]
    fn decode_snapshot_rejects_domain_invalid_project_state() {
        let project =
            Project::new(ProjectId::new(), "M0", Timestamp::now_utc()).expect("valid project");
        let mut value = serde_json::to_value(project).expect("project serializes");
        value["revision"] = json!(0);
        let serialized = serde_json::to_string(&value).expect("snapshot serializes");

        assert!(matches!(
            decode_snapshot::<Project>(&serialized, "project"),
            Err(StorageError::SnapshotInvariant {
                aggregate: "project",
                ..
            })
        ));
    }
}
