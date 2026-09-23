use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ActorId, ArtifactId, CommandId, DomainError, EmployeeId, EventId, PipelineId,
    PipelineVersionId, ProjectId, TaskId, Timestamp,
};

/// Class of an actor recorded in the canonical audit history.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// A human operator.
    Human,
    /// A scoped Employee acting through a valid Run or resolution assignment.
    Employee,
    /// The Project's system-management actor.
    SystemManager,
    /// The deterministic Forge Core.
    Core,
    /// The local Supervisor reporting observed state.
    Supervisor,
}

/// Stable identity and class of an actor responsible for a domain change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    id: ActorId,
    kind: ActorKind,
}

impl Actor {
    /// Creates a human actor reference.
    #[must_use]
    pub const fn human(id: ActorId) -> Self {
        Self {
            id,
            kind: ActorKind::Human,
        }
    }

    /// Creates an Employee actor reference.
    #[must_use]
    pub const fn employee(id: ActorId) -> Self {
        Self {
            id,
            kind: ActorKind::Employee,
        }
    }

    /// Creates a Project system-manager actor reference.
    #[must_use]
    pub const fn system_manager(id: ActorId) -> Self {
        Self {
            id,
            kind: ActorKind::SystemManager,
        }
    }

    /// Creates a Forge Core actor reference.
    #[must_use]
    pub const fn core(id: ActorId) -> Self {
        Self {
            id,
            kind: ActorKind::Core,
        }
    }

    /// Creates a Supervisor actor reference.
    #[must_use]
    pub const fn supervisor(id: ActorId) -> Self {
        Self {
            id,
            kind: ActorKind::Supervisor,
        }
    }

    /// Returns the stable actor identity.
    #[must_use]
    pub const fn id(self) -> ActorId {
        self.id
    }

    /// Returns the actor's authority class.
    #[must_use]
    pub const fn kind(self) -> ActorKind {
        self.kind
    }

    pub(crate) fn validate_snapshot(self) -> Result<(), DomainError> {
        self.id.validate_v7("actor.id")
    }
}

/// Type of aggregate associated with a [`DomainEvent`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateType {
    /// A Project aggregate.
    Project,
    /// A Task aggregate.
    Task,
    /// An Employee aggregate.
    Employee,
    /// An Artifact aggregate.
    Artifact,
    /// A Pipeline aggregate.
    Pipeline,
    /// An immutable Pipeline-version aggregate.
    PipelineVersion,
}

/// Strongly typed reference to the aggregate affected by a domain Event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "aggregate_type",
    content = "aggregate_id",
    rename_all = "snake_case"
)]
pub enum AggregateRef {
    /// A Project aggregate.
    Project(ProjectId),
    /// A Task aggregate.
    Task(TaskId),
    /// An Employee aggregate.
    Employee(EmployeeId),
    /// An Artifact aggregate.
    Artifact(ArtifactId),
    /// A Pipeline aggregate.
    Pipeline(PipelineId),
    /// An immutable Pipeline-version aggregate.
    PipelineVersion(PipelineVersionId),
}

impl AggregateRef {
    /// Returns the aggregate's category.
    #[must_use]
    pub const fn aggregate_type(self) -> AggregateType {
        match self {
            Self::Project(_) => AggregateType::Project,
            Self::Task(_) => AggregateType::Task,
            Self::Employee(_) => AggregateType::Employee,
            Self::Artifact(_) => AggregateType::Artifact,
            Self::Pipeline(_) => AggregateType::Pipeline,
            Self::PipelineVersion(_) => AggregateType::PipelineVersion,
        }
    }

    fn validate_snapshot(self) -> Result<(), DomainError> {
        match self {
            Self::Project(id) => id.validate_v7("event.aggregate.project_id"),
            Self::Task(id) => id.validate_v7("event.aggregate.task_id"),
            Self::Employee(id) => id.validate_v7("event.aggregate.employee_id"),
            Self::Artifact(id) => id.validate_v7("event.aggregate.artifact_id"),
            Self::Pipeline(id) => id.validate_v7("event.aggregate.pipeline_id"),
            Self::PipelineVersion(id) => id.validate_v7("event.aggregate.pipeline_version_id"),
        }
    }
}

/// Vocabulary for immutable domain changes and bounded rejected-runtime audit facts.
///
/// The payload remains versioned structured data so new consumers can evolve
/// independently while the Event kind keeps audit and routing semantics bounded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainEventKind {
    /// Semantic jobs are independent of Task execution and authority.
    SystemJobChanged,
    /// A human or authorized Manager revised canonical project knowledge.
    KnowledgePageChanged,
    /// A scoped system job produced or withdrew non-authoritative derived memory.
    DerivedMemoryChanged,
    /// A fenced Employee requested a new immutable tool-supplied context snapshot.
    KnowledgeContextRefreshed,
    FindingReported,
    FindingTriaged,
    FindingPromoted,
    /// One immutable deferred Task-resume instruction or its terminal result.
    TaskResumeScheduleChanged,
    ResolverRouteConfigured,
    EscalationRaised,
    ResolutionAssigned,
    ResolutionSubmitted,
    EscalationRerouted,
    /// An operator registered an immutable local source allowlist entry.
    ProjectRepositoryRegistered,
    /// A draft Task pinned its repository, initial base and persistent surface.
    TaskGitRepositoryBound,
    /// A future-only source selection changed independently from the Task revision.
    TaskGitSourcePolicyChanged,
    /// Explicit selected-file capture/import was requested or finished.
    TaskFileSnapshotChanged,
    /// Future Run inputs changed independently of Task lifecycle.
    TaskFileInputAttached,
    /// A Project-scoped Employee conversation was opened.
    EmployeeThreadOpened,
    /// An immutable input message was accepted for delivery.
    EmployeeMessageSent,
    EmployeeMessageAcknowledged,
    EmployeeMessageAnswered,
    EmployeeMessageRequirementWaived,
    /// A human explicitly configured the Project's host reboot behavior.
    BootRecoveryPolicyConfigured,
    /// Core reconciled a changed local execution host generation.
    ProjectRecoveryReconciled,
    /// Management accepted a recovery assessment without inventing a stage result.
    RunRecoveryAssessmentAccepted,
    /// A redacted technical evidence receipt was retained or uploaded.
    RunEvidenceRecorded,
    /// Provider-reported usage/exit diagnostics, never Task completion proof.
    RunRuntimeReported,
    /// A Run-scoped proxy key intent, issuance or revocation was recorded.
    RunProxyKeyChanged,
    /// Operator selected a validated runtime snapshot for future work.
    EmployeeRuntimeConfigured,
    /// Owner registered a new immutable project command version.
    ProjectHookConfigured,
    /// Runtime uncertainty or failure requires an explicit management decision.
    RunIncidentRaised,
    /// A credential was enrolled or its encrypted auth version changed.
    CredentialUpdated,
    /// A worker reported activity without changing task lifecycle.
    RunProgressReported,
    /// A Project was created with its initial policy catalog.
    ProjectCreated,
    /// Project Task-property schema was replaced before any Task existed.
    TaskPropertySchemaConfigured,
    /// Project dispatch was opened by an authorized manager command.
    ProjectExecutionStarted,
    /// Project dispatch was stopped by an authorized manager command.
    ProjectExecutionStopped,
    /// A named Pipeline catalog entry was created.
    PipelineCreated,
    /// An immutable Pipeline version was published.
    PipelineVersionPublished,
    /// The default changed without migrating historical Tasks.
    PipelineDefaultVersionChanged,
    /// Soft deletion preserves all versions and Task bindings.
    PipelineDeleted,
    /// A Task was created in draft.
    TaskCreated,
    /// A draft Task specification was amended.
    TaskAmended,
    /// A Task execution contract was approved.
    TaskApproved,
    /// The Task entered its first active Pipeline stage.
    TaskStarted,
    /// A Task entered a typed waiting state.
    TaskWaiting,
    /// A Task resumed after all active wait conditions resolved.
    TaskResumed,
    /// A Task completed its terminal success condition.
    TaskCompleted,
    /// A Task was cancelled with a catalog reason.
    TaskCancelled,
    /// A Task priority level changed.
    TaskPriorityChanged,
    /// A directed Task dependency was added after graph-cycle validation.
    TaskDependencyCreated,
    /// A directed Task dependency was removed.
    TaskDependencyRemoved,
    /// A Pipeline outcome moved a Task to another declared stage.
    TaskStageAdvanced,
    /// A finite Pipeline stage-entry budget was exhausted.
    TaskRetryExhausted,
    /// Immutable evidence was attached to a Task.
    TaskArtifactAttached,
    /// A writer proposed a Git outcome; no stage transition is implied.
    GitStageProposed,
    /// Exact post-quiescence Git inspection was retained.
    GitCandidateInspected,
    /// Audited deterministic Git integration intent, observation or management action.
    GitIntegrationChanged,
    /// A Pipeline-authorized Employee assessed an exact candidate revision.
    CandidateReviewed,
    /// Git acceptance requires an explicit management decision.
    GitProposalNeedsAttention,
    /// An immutable Artifact was submitted.
    ArtifactCreated,
    /// Core durably issued a fenced Lease and fake Run for queued work.
    RunProvisioned,
    /// Employee explicitly completed its bounded conversation assignment.
    CommunicationCompleted,
    /// Operator explicitly admitted another attempt of a held conversation.
    CommunicationRetryRequested,
    RuntimeInputRequested,
    RuntimeInputObserved,
    /// Core accepted a fenced runtime observation.
    RunObserved,
    /// Core rejected a stale runtime observation without mutating the Run.
    RunObservationIgnored,
    /// Core requested graceful or forceful stop of a fenced Run.
    RunStopRequested,
    /// The scoped Tool Gateway committed a capability grant or progress signal.
    ToolGatewayCallAllowed,
    /// The scoped Tool Gateway rejected a request without applying its action.
    ToolGatewayCallDenied,
    /// An Employee identity was created.
    EmployeeCreated,
    /// Employee catalog configuration changed for future assignments.
    EmployeeAmended,
    /// An Employee became eligible for new work.
    EmployeeEnabled,
    /// An Employee was disabled for new work.
    EmployeeDisabled,
    /// A manager stopped an Employee without claiming physical quiescence.
    EmployeeStopRequested,
    /// A manager requested an individual Task pause and explicit stop policy.
    TaskPauseRequested,
    /// One-shot next-Run Employee intent was created, held, consumed or cleared.
    TaskDispatchConstraintChanged,
    /// An Employee was retired while retaining its history.
    EmployeeRetired,
}

/// Validated input used to create one immutable [`DomainEvent`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainEventInput {
    /// Owning Project.
    pub project_id: ProjectId,
    /// Aggregate changed by the command.
    pub aggregate: AggregateRef,
    /// Aggregate revision after the change.
    pub aggregate_revision: u64,
    /// Bounded event vocabulary member.
    pub kind: DomainEventKind,
    /// Actor responsible for the command.
    pub actor: Actor,
    /// Idempotency identity of the named command.
    pub command_id: CommandId,
    /// Optional human-readable change reason.
    pub reason: Option<String>,
    /// Authoritative occurrence time.
    pub occurred_at: Timestamp,
    /// Version of the event envelope and payload schema.
    pub schema_version: u16,
    /// Structured payload. It must be a JSON object.
    pub payload: Value,
}

/// Immutable audit record created only after a successful named command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainEvent {
    id: EventId,
    project_id: ProjectId,
    aggregate: AggregateRef,
    aggregate_revision: u64,
    kind: DomainEventKind,
    actor: Actor,
    command_id: CommandId,
    reason: Option<String>,
    occurred_at: Timestamp,
    schema_version: u16,
    payload: Value,
}

impl DomainEvent {
    /// Creates a versioned immutable domain Event.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] when the aggregate revision or
    /// schema version is zero, or when `payload` is not a JSON object.
    pub fn new(id: EventId, input: DomainEventInput) -> Result<Self, DomainError> {
        if input.aggregate_revision == 0 {
            return Err(DomainError::InvalidValue {
                field: "event.aggregate_revision",
                reason: "must be greater than zero".to_owned(),
            });
        }
        if input.schema_version == 0 {
            return Err(DomainError::InvalidValue {
                field: "event.schema_version",
                reason: "must be greater than zero".to_owned(),
            });
        }
        if !input.payload.is_object() {
            return Err(DomainError::InvalidValue {
                field: "event.payload",
                reason: "must be a JSON object".to_owned(),
            });
        }

        Ok(Self {
            id,
            project_id: input.project_id,
            aggregate: input.aggregate,
            aggregate_revision: input.aggregate_revision,
            kind: input.kind,
            actor: input.actor,
            command_id: input.command_id,
            reason: input.reason,
            occurred_at: input.occurred_at,
            schema_version: input.schema_version,
            payload: input.payload,
        })
    }

    /// Returns the immutable Event ID.
    #[must_use]
    pub const fn id(&self) -> EventId {
        self.id
    }

    /// Returns the owning Project.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the affected aggregate.
    #[must_use]
    pub const fn aggregate(&self) -> AggregateRef {
        self.aggregate
    }

    /// Returns the aggregate's revision after this change.
    #[must_use]
    pub const fn aggregate_revision(&self) -> u64 {
        self.aggregate_revision
    }

    /// Returns the bounded Event vocabulary member.
    #[must_use]
    pub const fn kind(&self) -> DomainEventKind {
        self.kind
    }

    /// Returns the actor responsible for the change.
    #[must_use]
    pub const fn actor(&self) -> Actor {
        self.actor
    }

    /// Returns the idempotent command that caused the Event.
    #[must_use]
    pub const fn command_id(&self) -> CommandId {
        self.command_id
    }

    /// Returns the optional human-readable reason.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Returns the authoritative occurrence timestamp.
    #[must_use]
    pub const fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }

    /// Returns the event-envelope schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns the structured payload without interpreting its semantics.
    #[must_use]
    pub fn payload(&self) -> &Value {
        &self.payload
    }

    /// Revalidates a deserialized event snapshot before it is replayed.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("event.id")?;
        self.project_id.validate_v7("event.project_id")?;
        self.aggregate.validate_snapshot()?;
        self.actor.validate_snapshot()?;
        self.command_id.validate_v7("event.command_id")?;
        Self::new(
            self.id,
            DomainEventInput {
                project_id: self.project_id,
                aggregate: self.aggregate,
                aggregate_revision: self.aggregate_revision,
                kind: self.kind,
                actor: self.actor,
                command_id: self.command_id,
                reason: self.reason.clone(),
                occurred_at: self.occurred_at,
                schema_version: self.schema_version,
                payload: self.payload.clone(),
            },
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Actor, AggregateRef, DomainEvent, DomainEventInput, DomainEventKind};
    use crate::{ActorId, CommandId, EventId, ProjectId, TaskId, Timestamp};

    #[test]
    fn event_rejects_a_non_object_payload() {
        let event = DomainEvent::new(
            EventId::new(),
            DomainEventInput {
                project_id: ProjectId::new(),
                aggregate: AggregateRef::Task(TaskId::new()),
                aggregate_revision: 1,
                kind: DomainEventKind::TaskCreated,
                actor: Actor::human(ActorId::new()),
                command_id: CommandId::new(),
                reason: None,
                occurred_at: Timestamp::now_utc(),
                schema_version: 1,
                payload: json!("not an object"),
            },
        );

        assert!(event.is_err());
    }
}
