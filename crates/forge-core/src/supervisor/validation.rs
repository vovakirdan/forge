//! Bounded decoding for untrusted Supervisor protocol values.

use std::collections::BTreeSet;

use forge_domain::{
    ArtifactId, ArtifactKind, CancellationReasonId, OutcomeKey, StageId, Timestamp,
};
use forge_protocol::{
    supervisor::v1::{
        ArtifactSubmission, ExecutorSubmission, ObservedRunEvent, RunEventKind,
        StageOutcomeSubmission, SupervisorHello, executor_submission,
    },
    wire::{
        MAX_ARTIFACT_METADATA_SERIALIZED_BYTES, MAX_COMMAND_AUDIT_DETAIL_CHARACTERS,
        MAX_INLINE_ARTIFACT_BODY_BYTES, MAX_TITLE_CHARACTERS, validate_artifact_metadata_value,
    },
};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use super::TransportRejection;

const PROTOCOL_MAJOR: u32 = 1;
const MAX_ID_BYTES: usize = 64;
const MAX_HOST_OR_BOOT_BYTES: usize = 128;
const MAX_OBSERVATION_DETAILS_BYTES: usize = 16 * 1024;
const MAX_OUTCOME_ARTIFACT_IDS: usize = 64;

/// Validated exact fence scope carried by an executor submission.
#[derive(Clone, Copy, Debug)]
pub(super) struct SubmissionScope {
    pub(super) message_id: Uuid,
    pub(super) run_id: Uuid,
    pub(super) lease_fencing_token: u64,
    pub(super) environment_epoch: u64,
    pub(super) sequence: u64,
    pub(super) ingress: SubmissionIngress,
}

/// Gateway messages have a separate durable idempotency ledger and never
/// consume a sequence in the Supervisor observation stream.
#[derive(Clone, Copy, Debug)]
pub(super) enum SubmissionIngress {
    Supervisor,
    Gateway { payload_hash: [u8; 32] },
}

impl SubmissionScope {
    pub(super) fn is_gateway(&self) -> bool {
        matches!(self.ingress, SubmissionIngress::Gateway { .. })
    }
}

/// Parsed runtime observation ready for canonical Core handling.
#[derive(Clone, Debug)]
pub(super) struct ParsedObservation {
    pub(super) kind: RunEventKind,
    pub(super) message_id: Uuid,
    pub(super) run_id: Uuid,
    pub(super) lease_fencing_token: u64,
    pub(super) environment_epoch: u64,
    pub(super) sequence: u64,
    pub(super) state: forge_storage::RunObservedState,
    pub(super) details: Value,
    pub(super) reported_at: Timestamp,
}

/// Parsed artifact evidence submission.
#[derive(Clone, Debug)]
pub(super) struct ParsedArtifactSubmission {
    pub(super) scope: SubmissionScope,
    pub(super) kind: ArtifactKind,
    pub(super) metadata: Value,
    pub(super) body: Value,
    pub(super) title: String,
}

/// Parsed Pipeline outcome submission.
#[derive(Clone, Debug)]
pub(super) struct ParsedStageOutcomeSubmission {
    pub(super) candidate_commit: Option<forge_domain::git::GitObjectId>,
    pub(super) scope: SubmissionScope,
    pub(super) stage_id: StageId,
    pub(super) outcome: OutcomeKey,
    pub(super) artifact_ids: BTreeSet<ArtifactId>,
    pub(super) artifact_submission_message_ids: Vec<Uuid>,
    pub(super) note: Option<String>,
    pub(super) cancellation_reason_id: Option<CancellationReasonId>,
}

/// One fully validated executor payload variant.
#[derive(Clone, Debug)]
pub(super) enum ParsedExecutorSubmission {
    Artifact(ParsedArtifactSubmission),
    StageOutcome(ParsedStageOutcomeSubmission),
}

/// Validates the initial stream handshake without assigning authority itself.
pub(super) fn validate_hello(hello: &SupervisorHello) -> Result<(), TransportRejection> {
    let _ = parse_uuid_v7("hello.message_id", &hello.message_id)?;
    let _ = parse_uuid_v7("supervisor_instance_id", &hello.supervisor_instance_id)?;
    if hello.protocol_major != PROTOCOL_MAJOR {
        return Err(TransportRejection::new(
            "unsupported_protocol_major",
            "Supervisor protocol major is not supported",
        ));
    }
    nonblank_bounded("host_id", &hello.host_id, MAX_HOST_OR_BOOT_BYTES)?;
    nonblank_bounded("boot_id", &hello.boot_id, MAX_HOST_OR_BOOT_BYTES)?;
    let _ = timestamp_from_unix_ms("started_at_unix_ms", hello.started_at_unix_ms)?;
    Ok(())
}

/// Decodes an observation without touching canonical state.
pub(super) fn parse_observation(
    observation: ObservedRunEvent,
) -> Result<ParsedObservation, TransportRejection> {
    let state = observed_state(observation.kind)?;
    Ok(ParsedObservation {
        kind: RunEventKind::try_from(observation.kind).map_err(|_| {
            TransportRejection::new("invalid_observation_kind", "invalid observation kind")
        })?,
        message_id: parse_uuid_v7("observed_run_event.message_id", &observation.message_id)?,
        run_id: parse_uuid_v7("observed_run_event.run_id", &observation.run_id)?,
        lease_fencing_token: positive(
            "observed_run_event.lease_fencing_token",
            observation.lease_fencing_token,
        )?,
        environment_epoch: positive(
            "observed_run_event.environment_epoch",
            observation.environment_epoch,
        )?,
        sequence: positive("observed_run_event.sequence", observation.sequence)?,
        state,
        details: json_object(
            "observed_run_event.details_json",
            &observation.details_json,
            MAX_OBSERVATION_DETAILS_BYTES,
        )?,
        reported_at: timestamp_from_unix_ms(
            "observed_run_event.occurred_at_unix_ms",
            observation.occurred_at_unix_ms,
        )?,
    })
}

/// Decodes an executor payload without touching canonical state.
pub(super) fn parse_executor_submission(
    submission: ExecutorSubmission,
) -> Result<ParsedExecutorSubmission, TransportRejection> {
    let scope = SubmissionScope {
        message_id: parse_uuid_v7("executor_submission.message_id", &submission.message_id)?,
        run_id: parse_uuid_v7("executor_submission.run_id", &submission.run_id)?,
        lease_fencing_token: positive(
            "executor_submission.lease_fencing_token",
            submission.lease_fencing_token,
        )?,
        environment_epoch: positive(
            "executor_submission.environment_epoch",
            submission.environment_epoch,
        )?,
        sequence: positive("executor_submission.sequence", submission.sequence)?,
        ingress: SubmissionIngress::Supervisor,
    };
    let _ = timestamp_from_unix_ms(
        "executor_submission.submitted_at_unix_ms",
        submission.submitted_at_unix_ms,
    )?;
    match submission.payload {
        Some(executor_submission::Payload::Artifact(artifact)) => {
            parse_artifact_submission(scope, artifact).map(ParsedExecutorSubmission::Artifact)
        }
        Some(executor_submission::Payload::StageOutcome(outcome)) => {
            parse_stage_outcome_submission(scope, outcome)
                .map(ParsedExecutorSubmission::StageOutcome)
        }
        None => Err(TransportRejection::new(
            "missing_submission_payload",
            "Executor submission has no payload",
        )),
    }
}

pub(super) fn parse_uuid_v7(field: &'static str, value: &str) -> Result<Uuid, TransportRejection> {
    if value.len() > MAX_ID_BYTES {
        return Err(TransportRejection::new(
            "invalid_identifier",
            "Supervisor identifier is too long",
        ));
    }
    let identifier = Uuid::parse_str(value).map_err(|_| {
        TransportRejection::new(
            "invalid_identifier",
            "Supervisor identifier must be a UUIDv7",
        )
    })?;
    if identifier.get_version_num() != 7 {
        return Err(TransportRejection::new(
            "invalid_identifier",
            "Supervisor identifier must be a UUIDv7",
        ));
    }
    let _ = field;
    Ok(identifier)
}

/// Keeps an acknowledgement correlation id bounded and valid.
pub(super) fn acknowledged_message_id(value: &str) -> String {
    parse_uuid_v7("acknowledged_message_id", value)
        .map(|identifier| identifier.to_string())
        .unwrap_or_default()
}

pub(super) fn parse_artifact_submission(
    scope: SubmissionScope,
    artifact: ArtifactSubmission,
) -> Result<ParsedArtifactSubmission, TransportRejection> {
    let metadata = json_object(
        "artifact_submission.metadata_json",
        &artifact.metadata_json,
        MAX_ARTIFACT_METADATA_SERIALIZED_BYTES,
    )?;
    validate_artifact_metadata_value(&metadata).map_err(|_| {
        TransportRejection::new(
            "invalid_artifact_metadata",
            "Artifact metadata exceeds the M0 structural policy",
        )
    })?;
    if artifact.title.chars().count() > MAX_TITLE_CHARACTERS || artifact.title.trim().is_empty() {
        return Err(TransportRejection::new(
            "invalid_artifact_title",
            "Artifact title must be non-blank and within the M0 limit",
        ));
    }
    Ok(ParsedArtifactSubmission {
        scope,
        kind: ArtifactKind::new(artifact.artifact_kind).map_err(|_| {
            TransportRejection::new("invalid_artifact_kind", "Artifact kind is not a stable key")
        })?,
        metadata,
        body: json_value(
            "artifact_submission.body_json",
            &artifact.body_json,
            MAX_INLINE_ARTIFACT_BODY_BYTES,
        )?,
        title: artifact.title,
    })
}

pub(super) fn parse_stage_outcome_submission(
    scope: SubmissionScope,
    outcome: StageOutcomeSubmission,
) -> Result<ParsedStageOutcomeSubmission, TransportRejection> {
    if outcome.artifact_ids.len() > MAX_OUTCOME_ARTIFACT_IDS
        || outcome.artifact_submission_message_ids.len() > MAX_OUTCOME_ARTIFACT_IDS
    {
        return Err(TransportRejection::new(
            "too_many_outcome_artifacts",
            "Stage outcome exceeds the M0 artifact-reference limit",
        ));
    }
    let artifact_ids = unique_artifact_ids(&outcome.artifact_ids)?;
    let artifact_submission_message_ids =
        unique_message_ids(&outcome.artifact_submission_message_ids)?;
    let note = (!outcome.note.is_empty())
        .then_some(outcome.note)
        .map(|note| {
            if note.chars().count() > MAX_COMMAND_AUDIT_DETAIL_CHARACTERS {
                Err(TransportRejection::new(
                    "outcome_note_too_long",
                    "Stage outcome note exceeds the M0 limit",
                ))
            } else {
                Ok(note)
            }
        })
        .transpose()?;
    let cancellation_reason_id = (!outcome.cancellation_reason_key.is_empty())
        .then(|| CancellationReasonId::new(outcome.cancellation_reason_key))
        .transpose()
        .map_err(|_| {
            TransportRejection::new(
                "invalid_cancellation_reason",
                "Cancellation reason is not a stable key",
            )
        })?;
    Ok(ParsedStageOutcomeSubmission {
        candidate_commit: (!outcome.candidate_commit.is_empty())
            .then(|| forge_domain::git::GitObjectId::new(outcome.candidate_commit))
            .transpose()
            .map_err(|_| {
                TransportRejection::new(
                    "invalid_candidate_commit",
                    "candidate commit must be a full Git object identity",
                )
            })?,
        scope,
        stage_id: StageId::new(outcome.stage_id).map_err(|_| {
            TransportRejection::new("invalid_stage_id", "Stage id is not a stable key")
        })?,
        outcome: OutcomeKey::new(outcome.outcome).map_err(|_| {
            TransportRejection::new("invalid_outcome", "Outcome is not a stable key")
        })?,
        artifact_ids,
        artifact_submission_message_ids,
        note,
        cancellation_reason_id,
    })
}

fn unique_artifact_ids(values: &[String]) -> Result<BTreeSet<ArtifactId>, TransportRejection> {
    let mut parsed = BTreeSet::new();
    for value in values {
        if !parsed.insert(ArtifactId::from(parse_uuid_v7("artifact_ids", value)?)) {
            return Err(TransportRejection::new(
                "duplicate_artifact_id",
                "Stage outcome repeats an artifact id",
            ));
        }
    }
    Ok(parsed)
}

fn unique_message_ids(values: &[String]) -> Result<Vec<Uuid>, TransportRejection> {
    let mut parsed = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let message_id = parse_uuid_v7("artifact_submission_message_ids", value)?;
        if !seen.insert(message_id) {
            return Err(TransportRejection::new(
                "duplicate_artifact_submission_message_id",
                "Stage outcome repeats an artifact submission id",
            ));
        }
        parsed.push(message_id);
    }
    Ok(parsed)
}

fn observed_state(kind: i32) -> Result<forge_storage::RunObservedState, TransportRejection> {
    match RunEventKind::try_from(kind).ok() {
        Some(RunEventKind::Provisioning) => Ok(forge_storage::RunObservedState::Provisioning),
        Some(RunEventKind::Running | RunEventKind::Heartbeat) => {
            Ok(forge_storage::RunObservedState::Running)
        }
        Some(RunEventKind::Stopping) => Ok(forge_storage::RunObservedState::Stopping),
        Some(RunEventKind::Stopped) => Ok(forge_storage::RunObservedState::Stopped),
        Some(
            RunEventKind::StartFailed
            | RunEventKind::ProviderFailed
            | RunEventKind::BudgetExhausted
            | RunEventKind::PolicyDenied,
        ) => Ok(forge_storage::RunObservedState::Failed),
        Some(RunEventKind::EnvironmentLost) => Ok(forge_storage::RunObservedState::Lost),
        Some(RunEventKind::Unspecified) | None => Err(TransportRejection::new(
            "invalid_observation_kind",
            "Observed Run event kind is not supported",
        )),
    }
}

fn json_object(
    field: &'static str,
    raw: &str,
    maximum_bytes: usize,
) -> Result<Value, TransportRejection> {
    let value = json_value(field, raw, maximum_bytes)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(TransportRejection::new(
            "invalid_json_shape",
            "Supervisor JSON value must be an object",
        ))
    }
}

fn json_value(
    _field: &'static str,
    raw: &str,
    maximum_bytes: usize,
) -> Result<Value, TransportRejection> {
    if raw.len() > maximum_bytes {
        return Err(TransportRejection::new(
            "json_too_large",
            "Supervisor JSON value exceeds the M0 byte limit",
        ));
    }
    serde_json::from_str(raw)
        .map_err(|_| TransportRejection::new("invalid_json", "Supervisor JSON value is malformed"))
}

fn timestamp_from_unix_ms(
    _field: &'static str,
    milliseconds: i64,
) -> Result<Timestamp, TransportRejection> {
    let nanoseconds = i128::from(milliseconds)
        .checked_mul(1_000_000)
        .ok_or_else(|| {
            TransportRejection::new("invalid_timestamp", "Supervisor timestamp is out of range")
        })?;
    OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
        .map(Timestamp::from_offset_date_time)
        .map_err(|_| {
            TransportRejection::new("invalid_timestamp", "Supervisor timestamp is out of range")
        })
}

fn nonblank_bounded(
    _field: &'static str,
    value: &str,
    maximum_bytes: usize,
) -> Result<(), TransportRejection> {
    if value.trim().is_empty() || value.len() > maximum_bytes {
        return Err(TransportRejection::new(
            "invalid_supervisor_identity",
            "Supervisor identity field is blank or too long",
        ));
    }
    Ok(())
}

fn positive(_field: &'static str, value: u64) -> Result<u64, TransportRejection> {
    if value == 0 {
        return Err(TransportRejection::new(
            "invalid_fence_scope",
            "Supervisor Run fence scope must be positive",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use forge_protocol::supervisor::v1::{ObservedRunEvent, RunEventKind};
    use uuid::Uuid;

    use super::parse_observation;

    #[test]
    fn observation_rejects_non_object_details_before_storage() {
        let observation = ObservedRunEvent {
            message_id: Uuid::now_v7().to_string(),
            run_id: Uuid::now_v7().to_string(),
            lease_fencing_token: 1,
            environment_epoch: 1,
            sequence: 1,
            occurred_at_unix_ms: 1,
            kind: RunEventKind::Running as i32,
            details_json: "[]".to_owned(),
        };

        let result = parse_observation(observation);

        assert!(result.is_err());
    }
}
