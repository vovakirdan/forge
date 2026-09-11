//! Shared HTTP/JSON and SSE envelopes for the local Forge API.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The current public local API version.
pub const API_VERSION: &str = "v1";

/// The current envelope schema version for durable and SSE events.
pub const EVENT_SCHEMA_VERSION: u16 = 1;

/// M0 version of the opaque RunSpec JSON carried to Supervisor.
///
/// A [`crate::supervisor::v1::ProvisionRun`] carries this value separately from
/// its JSON payload so a Supervisor can reject an unsupported shape before it
/// attempts to provision a Run. A zero value is never a valid Forge RunSpec.
pub const M0_RUN_SPEC_VERSION: u32 = 1;

/// Maximum UTF-8 byte length of an M0 inline Artifact JSON body.
///
/// The limit applies to the serialized JSON document, including JSON syntax,
/// rather than to a character count. It is shared by HTTP command payloads and
/// Supervisor `ArtifactSubmission` messages. Binary or larger evidence must
/// use an object reference once that storage mode is introduced after M0.
pub const MAX_INLINE_ARTIFACT_BODY_BYTES: usize = 256 * 1024;

/// Maximum Unicode scalar-value length of a Task or Artifact title.
///
/// Core repeats this boundary while converting transport data into domain
/// values. The wire constant keeps all public transports aligned before that
/// conversion happens.
pub const MAX_TITLE_CHARACTERS: usize = 240;

/// Maximum byte length of a stable Project-defined key.
///
/// Stable keys are lowercase ASCII, begin with a letter, and then contain only
/// lowercase ASCII letters, digits, or underscores.
pub const MAX_STABLE_KEY_BYTES: usize = 64;

/// Maximum Unicode scalar-value length of an optional command audit detail.
///
/// This applies to a cancellation note and to the optional reason on project
/// execution start or stop. It is measured as Rust `char` values, which aligns
/// with the OpenAPI `maxLength` contract rather than UTF-8 byte length.
pub const MAX_COMMAND_AUDIT_DETAIL_CHARACTERS: usize = 10_000;

/// Maximum serialized UTF-8 byte length of an Artifact metadata object.
///
/// Metadata is deliberately smaller than an inline Artifact body: it is
/// indexed and rendered as structured context, not used for bulk evidence.
pub const MAX_ARTIFACT_METADATA_SERIALIZED_BYTES: usize = 16 * 1024;

/// Maximum nested object/array depth in Artifact metadata, including its root
/// object as depth one.
pub const MAX_ARTIFACT_METADATA_DEPTH: usize = 8;

/// Maximum total object members and array elements in Artifact metadata.
///
/// The count covers every nested container, not only top-level keys.
pub const MAX_ARTIFACT_METADATA_ENTRIES: usize = 64;

/// A structural Artifact metadata refusal that every public ingress may reuse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactMetadataError {
    /// Metadata was not represented as a JSON object.
    MustBeObject,
    /// Serializing metadata into its canonical JSON representation failed.
    CannotSerialize,
    /// Metadata exceeded its serialized-byte budget.
    TooLarge {
        /// Configured byte limit.
        maximum: usize,
        /// Measured serialized byte length.
        actual: usize,
    },
    /// Metadata exceeded its nested-container limit.
    TooDeep {
        /// Configured nesting limit.
        maximum: usize,
        /// Measured container depth.
        actual: usize,
    },
    /// Metadata contained too many object members or array elements.
    TooManyEntries {
        /// Configured entry limit.
        maximum: usize,
        /// Measured total entry count.
        actual: usize,
    },
}

impl fmt::Display for ArtifactMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MustBeObject => formatter.write_str("must be a JSON object"),
            Self::CannotSerialize => formatter.write_str("cannot be serialized as JSON"),
            Self::TooLarge { maximum, actual } => write!(
                formatter,
                "has {actual} serialized bytes; at most {maximum} are allowed"
            ),
            Self::TooDeep { maximum, actual } => write!(
                formatter,
                "has nested container depth {actual}; at most {maximum} is allowed"
            ),
            Self::TooManyEntries { maximum, actual } => write!(
                formatter,
                "has {actual} total entries; at most {maximum} are allowed"
            ),
        }
    }
}

impl std::error::Error for ArtifactMetadataError {}

/// Validates an Artifact metadata object against the shared M0 ingress policy.
///
/// HTTP application parsing uses this function directly. Supervisor and proto
/// adapters that still hold a [`Value`] can use
/// [`validate_artifact_metadata_value`] without reconstructing an object.
///
/// # Errors
///
/// Returns [`ArtifactMetadataError`] when metadata exceeds its serialized size,
/// nested-container depth, or total-entry budget.
pub fn validate_artifact_metadata(
    metadata: &Map<String, Value>,
) -> Result<(), ArtifactMetadataError> {
    let serialized =
        serde_json::to_vec(metadata).map_err(|_| ArtifactMetadataError::CannotSerialize)?;
    if serialized.len() > MAX_ARTIFACT_METADATA_SERIALIZED_BYTES {
        return Err(ArtifactMetadataError::TooLarge {
            maximum: MAX_ARTIFACT_METADATA_SERIALIZED_BYTES,
            actual: serialized.len(),
        });
    }

    let mut total_entries = 0;
    validate_metadata_object(metadata, 1, &mut total_entries)
}

/// Validates a `Value` received by an adapter as Artifact metadata.
///
/// This avoids a protocol-specific duplicate of the HTTP metadata policy.
///
/// # Errors
///
/// Returns [`ArtifactMetadataError::MustBeObject`] when the value is not an
/// object, or the same structural errors as [`validate_artifact_metadata`].
pub fn validate_artifact_metadata_value(value: &Value) -> Result<(), ArtifactMetadataError> {
    let Value::Object(metadata) = value else {
        return Err(ArtifactMetadataError::MustBeObject);
    };
    validate_artifact_metadata(metadata)
}

fn validate_metadata_node(
    value: &Value,
    depth: usize,
    total_entries: &mut usize,
) -> Result<(), ArtifactMetadataError> {
    match value {
        Value::Object(values) => validate_metadata_object(values, depth, total_entries),
        Value::Array(values) => validate_metadata_array(values, depth, total_entries),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

fn validate_metadata_object(
    values: &Map<String, Value>,
    depth: usize,
    total_entries: &mut usize,
) -> Result<(), ArtifactMetadataError> {
    validate_metadata_entries(values.values(), values.len(), depth, total_entries)
}

fn validate_metadata_array(
    values: &[Value],
    depth: usize,
    total_entries: &mut usize,
) -> Result<(), ArtifactMetadataError> {
    validate_metadata_entries(values.iter(), values.len(), depth, total_entries)
}

fn validate_metadata_entries<'a>(
    values: impl Iterator<Item = &'a Value>,
    count: usize,
    depth: usize,
    total_entries: &mut usize,
) -> Result<(), ArtifactMetadataError> {
    if depth > MAX_ARTIFACT_METADATA_DEPTH {
        return Err(ArtifactMetadataError::TooDeep {
            maximum: MAX_ARTIFACT_METADATA_DEPTH,
            actual: depth,
        });
    }
    *total_entries = total_entries.saturating_add(count);
    if *total_entries > MAX_ARTIFACT_METADATA_ENTRIES {
        return Err(ArtifactMetadataError::TooManyEntries {
            maximum: MAX_ARTIFACT_METADATA_ENTRIES,
            actual: *total_entries,
        });
    }
    for child in values {
        validate_metadata_node(child, depth + 1, total_entries)?;
    }
    Ok(())
}

/// M0 named commands accepted by `POST /v1/commands/{name}`.
///
/// The HTTP path selects this command name; Core then deserializes and
/// validates the corresponding typed payload. The JSON `payload` field remains
/// opaque here so this crate does not duplicate domain command types.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandName {
    ConfigureSystemJobs,
    RequestTaskSummary,
    RequestEmployeeOnboarding,
    RetrySystemJob,
    SkipEmployeeOnboarding,
    /// Create or edit an unpublished canonical knowledge page.
    AuthorKnowledgePage,
    /// Explicitly publish a canonical knowledge revision.
    PublishKnowledgePage,
    /// Replace a published revision while preserving its immutable history.
    SupersedeKnowledgePage,
    /// Withdraw a canonical page from future contexts and retrieval.
    WithdrawKnowledgePage,
    ReportFinding,
    TriageFinding,
    PromoteFinding,
    /// Register an immutable local Project repository allowlist entry.
    RegisterProjectRepository,
    /// Pin a draft Task to a registered source and explicit initial commit.
    BindTaskGitRepository,
    /// Configure the source selection for future Task writer Runs only.
    SetTaskGitSourcePolicy,
    /// Configure audited Project reboot behavior.
    ConfigureBootRecoveryPolicy,
    /// Accept a candidate assessment for one interrupted execution.
    AcceptRunRecoveryAssessment,
    /// Explicit bounded conversation retry after physical quiescence.
    RetryCommunication,
    /// Authorize a retained exact Git intent only after a new read-only reconciliation.
    RetryGitIntegration,
    /// Accept a previously applied Git fact without repeating its physical effect.
    AcceptGitIntegrationResult,
    /// Create a Project with the caller-reserved Project identity.
    CreateProject,
    /// Create a Pipeline and its initial immutable PipelineVersion.
    CreatePipeline,
    /// Publish a complete immutable graph under an existing Pipeline.
    PublishPipelineVersion,
    /// Select an already published graph for future Task bindings.
    SetPipelineDefaultVersion,
    /// Soft-delete the catalog entry without deleting versions.
    DeletePipeline,
    /// Create an enabled Employee identity in a Project.
    CreateEmployee,
    /// Amend an Employee catalog revision without changing existing Runs.
    AmendEmployee,
    /// Enable an Employee for future assignments.
    EnableEmployee,
    /// Disable future Employee assignments without implicitly stopping Runs.
    DisableEmployee,
    /// Permanently retire an Employee while retaining existing Run history.
    RetireEmployee,
    /// Disable future admission and stop this Employee's existing executions.
    StopEmployee,
    /// Pin one eligible Employee for the next Run of this Task stage visit.
    SetNextRunEmployee,
    /// Clear an unused Employee admission constraint without restarting work.
    ClearNextRunEmployee,
    /// Store one explicitly scoped Task-resume alarm.
    ScheduleTaskResume,
    ConfigureResolverRoute,
    RaiseEscalation,
    SubmitHumanResolution,
    RerouteEscalation,
    /// Cancel one pending alarm without changing its Task.
    CancelTaskResume,
    /// Open a durable Employee conversation with optional Task context.
    OpenEmployeeThread,
    /// Append an immutable message and its delivery intent.
    SendEmployeeMessage,
    WaiveMessageRequirement,
    /// Create a draft Task in a Project.
    CreateTask,
    /// Update mutable draft fields before approval.
    AmendDraft,
    /// Make a draft Task eligible for its Pipeline entry stage.
    ApproveTask,
    /// Stop a Task permanently with a catalogued cancellation reason.
    CancelTask,
    /// Resume a Task from a valid waiting condition.
    ResumeTask,
    /// Pause one Task and explicitly stop its assigned executions.
    PauseTask,
    /// Change the Task's configured priority value.
    SetTaskPriority,
    /// Add a hard Task dependency.
    CreateDependency,
    /// Remove a hard Task dependency.
    RemoveDependency,
    /// Open Project dispatch for eligible queued work.
    StartProjectExecution,
    /// Prevent new Project dispatch while preserving canonical history.
    StopProjectExecution,
    /// Record a human/resolver outcome at an external Pipeline stage.
    SubmitExternalStageOutcome,
    /// Assign a validated immutable runtime profile for future Employee Runs.
    ConfigureEmployeeRuntime,
    /// Register a new immutable version of an explicit provider-free project hook.
    ConfigureProjectHook,
    /// Import explicitly selected local files as immutable Task evidence.
    ImportTaskFileSnapshot,
    /// Capture explicitly selected files after the Task writer has stopped.
    CaptureTaskFileSnapshot,
    /// Supply one immutable snapshot to future Runs of another Task.
    AttachTaskFileInput,
    /// Enroll a new credential from an explicitly selected private local file.
    EnrollCredential,
}

/// Common body for a named local API command.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    /// Project that owns the target aggregate and command scope. For
    /// `create_project`, the caller reserves this UUIDv7 identity and Core
    /// requires `expected_revision` to be zero.
    pub project_id: String,
    /// Revision expected by the caller for the Project command boundary.
    pub expected_revision: u64,
    /// Command-specific JSON object validated by Core after the path selects a
    /// [`CommandName`].
    pub payload: Map<String, Value>,
}

/// Result returned after Core commits a named command or recognizes its retry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandReceipt {
    /// Stable id assigned to the accepted command.
    pub command_id: String,
    /// Whether this request changed canonical state or replayed a prior result.
    pub status: CommandStatus,
    /// Project revision after the original committed command.
    pub project_revision: u64,
    /// Events committed by the original command in canonical order.
    pub event_ids: Vec<String>,
    /// Primary resource created or changed when one is useful to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceReference>,
}

/// Idempotency-aware completion status for a command receipt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// Core committed this command during the current request.
    Applied,
    /// Core returned the durable result of a prior command with the same key.
    Replayed,
}

/// A stable reference to a resource returned by a command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReference {
    /// Wire-level resource kind, such as `task` or `dependency`.
    pub kind: String,
    /// Stable resource identifier.
    pub id: String,
}

/// Envelope returned by every local API refusal or failure.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    /// Typed error data safe to show to the local operator.
    pub error: ApiError,
}

/// Structured local API error data.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    /// Stable machine-readable classification.
    pub code: ApiErrorCode,
    /// Safe human-readable explanation; it never includes credentials or logs.
    pub message: String,
    /// Optional structured details specific to the error code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Map<String, Value>>,
    /// Correlation id for local diagnostics when one was assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Stable local API refusal and failure classifications.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    /// The request shape, header, or JSON encoding is invalid.
    InvalidRequest,
    /// A syntactically valid request fails structural validation.
    ValidationFailed,
    /// The referenced Project or resource does not exist in the caller scope.
    NotFound,
    /// The request conflicts with current canonical state.
    Conflict,
    /// The supplied expected revision no longer matches canonical state.
    StaleRevision,
    /// An idempotency key was reused for a different command payload.
    IdempotencyConflict,
    /// The server-derived actor lacks the required capability.
    Forbidden,
    /// Project execution is stopped and the command requires active dispatch.
    ExecutionStopped,
    /// The requested event cursor is malformed or cannot be replayed.
    CursorInvalid,
    /// A required local dependency is temporarily unavailable.
    Unavailable,
    /// An unexpected server failure occurred; diagnostics remain server-side.
    Internal,
}

/// Canonical event representation sent through SSE.
///
/// `payload` remains an event-type-specific JSON object. Consumers must branch
/// on `event_type` and reject a schema version they do not understand.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    /// Version of this event envelope and payload contract.
    pub schema_version: u16,
    /// Stable event id. UUIDv7 is expected.
    pub event_id: String,
    /// Project that owns the event and its monotonically increasing sequence.
    pub project_id: String,
    /// Canonical per-Project sequence used as the SSE event id and cursor.
    pub project_sequence: u64,
    /// Event type, for example `task.created` or `run.observed`.
    pub event_type: String,
    /// Aggregate materially changed by this event.
    pub aggregate: AggregateReference,
    /// Server timestamp encoded as an RFC 3339 UTC string.
    pub occurred_at: String,
    /// Server-derived actor that caused the accepted command.
    pub actor: ActorReference,
    /// Accepted command that committed this event, when the durable record has
    /// one. Canonical Core writes provide an id; `null` preserves historical
    /// durable records that predate command attribution.
    pub command_id: Option<String>,
    /// Optional human-readable audit reason kept separate from the
    /// event-type-specific payload. `null` means the durable Event had no
    /// audit reason.
    pub audit_reason: Option<String>,
    /// Event-type-specific, non-secret structured data.
    pub payload: Map<String, Value>,
}

/// Wire identity of an aggregate affected by an Event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateReference {
    /// Aggregate kind, such as `task`, `run`, or `pipeline_version`.
    pub kind: String,
    /// Stable aggregate identifier.
    pub id: String,
    /// Aggregate revision after the event when the aggregate is revisioned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

/// Actor class recorded on a canonical Event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// A human local operator.
    Human,
    /// A scoped Employee acting through an accepted execution authority.
    Employee,
    /// The Project's system-management actor.
    SystemManager,
    /// The deterministic Forge Core.
    Core,
    /// The local Supervisor reporting observed runtime state.
    Supervisor,
}

/// Wire identity of an actor recorded on an Event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorReference {
    /// Actor class: `human`, `employee`, `system_manager`, `core`, or
    /// `supervisor`.
    pub kind: ActorKind,
    /// Stable UUIDv7 actor identifier.
    pub id: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ActorKind, ArtifactMetadataError, CommandName, CommandRequest, EVENT_SCHEMA_VERSION,
        M0_RUN_SPEC_VERSION, MAX_ARTIFACT_METADATA_DEPTH, MAX_ARTIFACT_METADATA_ENTRIES,
        MAX_ARTIFACT_METADATA_SERIALIZED_BYTES, MAX_COMMAND_AUDIT_DETAIL_CHARACTERS,
        MAX_INLINE_ARTIFACT_BODY_BYTES, MAX_STABLE_KEY_BYTES, MAX_TITLE_CHARACTERS,
        validate_artifact_metadata, validate_artifact_metadata_value,
    };

    #[test]
    fn command_name_serializes_as_the_http_path_segment() {
        let actual = serde_json::to_string(&CommandName::SubmitExternalStageOutcome)
            .expect("test serialization should succeed");

        assert_eq!(actual, "\"submit_external_stage_outcome\"");
    }

    #[test]
    fn command_request_keeps_the_payload_for_core_validation() {
        let request = CommandRequest {
            project_id: "019...".to_owned(),
            expected_revision: 7,
            payload: json!({"task_id": "020...", "title": "Investigate"})
                .as_object()
                .expect("test fixture is an object")
                .clone(),
        };

        let encoded = serde_json::to_value(request).expect("test serialization should succeed");

        assert_eq!(encoded["payload"]["title"], "Investigate");
    }

    #[test]
    fn event_schema_version_starts_at_one() {
        assert_eq!(EVENT_SCHEMA_VERSION, 1);
    }

    #[test]
    fn local_operator_events_use_the_canonical_human_actor_kind() {
        let actual =
            serde_json::to_string(&ActorKind::Human).expect("test serialization should succeed");

        assert_eq!(actual, "\"human\"");
    }

    #[test]
    fn public_transport_bounds_are_stable() {
        assert_eq!(MAX_INLINE_ARTIFACT_BODY_BYTES, 262_144);
        assert_eq!(MAX_TITLE_CHARACTERS, 240);
        assert_eq!(MAX_STABLE_KEY_BYTES, 64);
        assert_eq!(MAX_COMMAND_AUDIT_DETAIL_CHARACTERS, 10_000);
        assert_eq!(MAX_ARTIFACT_METADATA_SERIALIZED_BYTES, 16_384);
        assert_eq!(MAX_ARTIFACT_METADATA_DEPTH, 8);
        assert_eq!(MAX_ARTIFACT_METADATA_ENTRIES, 64);
        assert_eq!(M0_RUN_SPEC_VERSION, 1);
    }

    #[test]
    fn artifact_metadata_policy_accepts_a_small_object() {
        let metadata = json!({"source": "human", "attempt": {"number": 1}});

        let result = validate_artifact_metadata_value(&metadata);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn artifact_metadata_policy_rejects_a_non_object_value() {
        let result = validate_artifact_metadata_value(&json!(["not", "metadata"]));

        assert_eq!(result, Err(ArtifactMetadataError::MustBeObject));
    }

    #[test]
    fn artifact_metadata_policy_rejects_more_than_the_entry_budget() {
        let metadata = (0..=MAX_ARTIFACT_METADATA_ENTRIES)
            .map(|index| (format!("field_{index}"), json!(index)))
            .collect();

        let result = validate_artifact_metadata(&metadata);

        assert!(matches!(
            result,
            Err(ArtifactMetadataError::TooManyEntries { .. })
        ));
    }

    #[test]
    fn artifact_metadata_policy_rejects_excessive_container_depth() {
        let mut nested = json!("leaf");
        for _ in 0..MAX_ARTIFACT_METADATA_DEPTH {
            nested = json!([nested]);
        }

        let result = validate_artifact_metadata_value(&json!({"nested": nested}));

        assert!(matches!(result, Err(ArtifactMetadataError::TooDeep { .. })));
    }

    #[test]
    fn artifact_metadata_policy_rejects_excessive_serialized_size() {
        let metadata = json!({"text": "x".repeat(MAX_ARTIFACT_METADATA_SERIALIZED_BYTES)});

        let result = validate_artifact_metadata_value(&metadata);

        assert!(matches!(
            result,
            Err(ArtifactMetadataError::TooLarge { .. })
        ));
    }

    #[test]
    fn command_request_rejects_unknown_top_level_fields() {
        let request = json!({
            "project_id": "019...",
            "expected_revision": 0,
            "payload": {},
            "unrecognized": true
        });

        let parsed = serde_json::from_value::<CommandRequest>(request);

        assert!(parsed.is_err());
    }
}
