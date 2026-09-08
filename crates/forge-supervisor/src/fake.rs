//! Deterministic M0 RunSpec parser and fake executor.

use std::{sync::Arc, time::Duration};

use forge_protocol::supervisor::v1::{
    ArtifactSubmission, ExecutorSubmission, ObservedRunEvent, ProvisionRun, RunEventKind,
    StageOutcomeSubmission, SupervisorToCore, executor_submission, supervisor_to_core,
};
use forge_protocol::wire::{
    ArtifactMetadataError, MAX_INLINE_ARTIFACT_BODY_BYTES, validate_artifact_metadata_value,
};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::{
    EventSink, M0_RUN_SPEC_VERSION, RunControl, SupervisorError, new_id, now_millis, send,
};

/// Executes an opaque provision request through the deterministic M0 simulator.
pub(crate) async fn execute_provision(
    provision: ProvisionRun,
    outbound: EventSink,
    control: Arc<RunControl>,
) -> Result<(), SupervisorError> {
    let mut sequence = RunSequence::default();
    let spec = match FakeRunSpec::parse(provision.run_spec_version, &provision.run_spec_json) {
        Ok(spec) => spec,
        Err(error) => {
            send_failure(&provision, &outbound, &mut sequence, error.reason_code()).await?;
            return Ok(());
        }
    };

    send_observation(
        &outbound,
        &provision,
        &mut sequence,
        RunEventKind::Provisioning,
        json!({"executor": "m0_fake"}),
    )
    .await?;
    if wait_or_stopped(&control, spec.delay).await {
        return send_stop_observations(&provision, &outbound, &mut sequence, "core_stop_requested")
            .await;
    }

    send_observation(
        &outbound,
        &provision,
        &mut sequence,
        RunEventKind::Running,
        json!({"executor": "m0_fake"}),
    )
    .await?;

    let mut artifact_submission_message_ids = Vec::with_capacity(spec.artifacts.len());
    for artifact in spec.artifacts {
        if wait_or_stopped(&control, spec.delay).await {
            return send_stop_observations(
                &provision,
                &outbound,
                &mut sequence,
                "core_stop_requested",
            )
            .await;
        }
        artifact_submission_message_ids
            .push(send_artifact(&outbound, &provision, &mut sequence, artifact).await?);
    }

    if wait_or_stopped(&control, spec.delay).await {
        return send_stop_observations(&provision, &outbound, &mut sequence, "core_stop_requested")
            .await;
    }
    send_stage_outcome(
        &outbound,
        &provision,
        &mut sequence,
        spec.stage_outcome,
        artifact_submission_message_ids,
    )
    .await?;
    send_stopped(&provision, &outbound, &mut sequence, "completed").await
}

/// Reports one terminal failure. `Failed` is terminal in the canonical Run
/// observation state machine, so it must never be followed by `Stopped`.
async fn send_failure(
    provision: &ProvisionRun,
    outbound: &EventSink,
    sequence: &mut RunSequence,
    reason_code: &str,
) -> Result<(), SupervisorError> {
    send_observation(
        outbound,
        provision,
        sequence,
        RunEventKind::StartFailed,
        json!({"reason_code": reason_code}),
    )
    .await
}

async fn send_stop_observations(
    provision: &ProvisionRun,
    outbound: &EventSink,
    sequence: &mut RunSequence,
    reason_code: &str,
) -> Result<(), SupervisorError> {
    send_observation(
        outbound,
        provision,
        sequence,
        RunEventKind::Stopping,
        json!({"reason_code": reason_code}),
    )
    .await?;
    send_stopped(provision, outbound, sequence, reason_code).await
}

async fn send_stopped(
    provision: &ProvisionRun,
    outbound: &EventSink,
    sequence: &mut RunSequence,
    reason_code: &str,
) -> Result<(), SupervisorError> {
    send_observation(
        outbound,
        provision,
        sequence,
        RunEventKind::Stopped,
        json!({"reason_code": reason_code}),
    )
    .await
}

async fn send_observation(
    outbound: &EventSink,
    provision: &ProvisionRun,
    sequence: &mut RunSequence,
    kind: RunEventKind,
    details: Value,
) -> Result<(), SupervisorError> {
    let event = ObservedRunEvent {
        message_id: new_id(),
        run_id: provision.run_id.clone(),
        lease_fencing_token: provision.lease_fencing_token,
        environment_epoch: provision.environment_epoch,
        sequence: sequence.next()?,
        occurred_at_unix_ms: now_millis(),
        kind: kind as i32,
        details_json: serde_json::to_string(&details)?,
    };
    send(
        outbound,
        SupervisorToCore {
            message: Some(supervisor_to_core::Message::ObservedRunEvent(event)),
        },
    )
    .await
}

async fn send_artifact(
    outbound: &EventSink,
    provision: &ProvisionRun,
    sequence: &mut RunSequence,
    artifact: FakeArtifactSpec,
) -> Result<String, SupervisorError> {
    let artifact = ArtifactSubmission {
        artifact_kind: artifact.kind,
        metadata_json: serde_json::to_string(&artifact.metadata)?,
        body_json: serde_json::to_string(&artifact.body)?,
        title: artifact.title,
    };
    send_submission(
        outbound,
        provision,
        sequence,
        executor_submission::Payload::Artifact(artifact),
    )
    .await
}

async fn send_stage_outcome(
    outbound: &EventSink,
    provision: &ProvisionRun,
    sequence: &mut RunSequence,
    outcome: FakeStageOutcome,
    artifact_submission_message_ids: Vec<String>,
) -> Result<(), SupervisorError> {
    let outcome = StageOutcomeSubmission {
        candidate_commit: String::new(),
        stage_id: provision.stage_id.clone(),
        outcome: outcome.outcome,
        // These are already-linked TaskHistory Artifact identities deliberately
        // named by the fake RunSpec, never generated from this Run's output.
        artifact_ids: outcome.artifact_ids,
        note: outcome.note.unwrap_or_default(),
        cancellation_reason_key: outcome.cancellation_reason_key.unwrap_or_default(),
        // Core owns canonical Artifact identities. It atomically resolves only
        // these explicit, same-scope ArtifactSubmission envelope identities.
        artifact_submission_message_ids,
    };
    send_submission(
        outbound,
        provision,
        sequence,
        executor_submission::Payload::StageOutcome(outcome),
    )
    .await
    .map(|_| ())
}

async fn send_submission(
    outbound: &EventSink,
    provision: &ProvisionRun,
    sequence: &mut RunSequence,
    payload: executor_submission::Payload,
) -> Result<String, SupervisorError> {
    let message_id = new_id();
    let submission = ExecutorSubmission {
        message_id: message_id.clone(),
        run_id: provision.run_id.clone(),
        lease_fencing_token: provision.lease_fencing_token,
        environment_epoch: provision.environment_epoch,
        sequence: sequence.next()?,
        submitted_at_unix_ms: now_millis(),
        payload: Some(payload),
    };
    send(
        outbound,
        SupervisorToCore {
            message: Some(supervisor_to_core::Message::ExecutorSubmission(submission)),
        },
    )
    .await?;
    Ok(message_id)
}

async fn wait_or_stopped(control: &RunControl, delay: Duration) -> bool {
    if control.stopped() {
        return true;
    }
    if delay.is_zero() {
        tokio::task::yield_now().await;
        return control.stopped();
    }

    tokio::select! {
        () = tokio::time::sleep(delay) => control.stopped(),
        () = control.stop_notified() => true,
    }
}

#[derive(Default)]
struct RunSequence(u64);

impl RunSequence {
    fn next(&mut self) -> Result<u64, SupervisorError> {
        self.0 = self
            .0
            .checked_add(1)
            .ok_or(SupervisorError::SequenceOverflow)?;
        Ok(self.0)
    }
}

/// Private JSON document accepted as M0 `ProvisionRun.run_spec_json`.
///
/// ```json
/// {
///   "artifacts": [{"kind":"change_set","title":"M0 change", "metadata":{}, "body":{}}],
///   "stage_outcome": {"outcome":"implemented", "note":"optional"},
///   "delay_ms": 0
/// }
/// ```
///
/// The supplied stage is intentionally ignored: this executor must echo the
/// stage pinned in `ProvisionRun`, never let opaque RunSpec change it.
///
/// `artifacts` deliberately contains no canonical Artifact identities. The
/// fake executor remembers each emitted `ArtifactSubmission` envelope ID and
/// explicitly places those IDs in the later `StageOutcomeSubmission`. An
/// optional `stage_outcome.artifact_ids` list is reserved for already-linked
/// TaskHistory evidence; it is not populated from the `artifacts` list.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeRunSpec {
    #[serde(default)]
    artifacts: Vec<FakeArtifactSpec>,
    stage_outcome: FakeStageOutcome,
    #[serde(rename = "delay_ms", default, deserialize_with = "deserialize_delay")]
    delay: Duration,
}

impl FakeRunSpec {
    fn parse(version: u32, json: &str) -> Result<Self, FakeRunSpecError> {
        if version != M0_RUN_SPEC_VERSION {
            return Err(FakeRunSpecError::UnsupportedVersion { received: version });
        }
        let spec: Self = serde_json::from_str(json)?;
        for (index, artifact) in spec.artifacts.iter().enumerate() {
            validate_artifact_contract(index, artifact)?;
        }
        if spec.stage_outcome.outcome.trim().is_empty() {
            return Err(FakeRunSpecError::BlankOutcome);
        }
        let unique_artifact_ids = spec
            .stage_outcome
            .artifact_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique_artifact_ids.len() != spec.stage_outcome.artifact_ids.len() {
            return Err(FakeRunSpecError::DuplicateTaskHistoryArtifactIds);
        }
        Ok(spec)
    }
}

fn validate_artifact_contract(
    index: usize,
    artifact: &FakeArtifactSpec,
) -> Result<(), FakeRunSpecError> {
    validate_artifact_metadata_value(&artifact.metadata).map_err(|source| match source {
        ArtifactMetadataError::MustBeObject => FakeRunSpecError::MetadataMustBeObject { index },
        source => FakeRunSpecError::InvalidArtifactMetadata { index, source },
    })?;

    let serialized_body = serde_json::to_vec(&artifact.body)
        .map_err(|_| FakeRunSpecError::ArtifactBodyCannotSerialize { index })?;
    if serialized_body.len() > MAX_INLINE_ARTIFACT_BODY_BYTES {
        return Err(FakeRunSpecError::ArtifactBodyTooLarge { index });
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeArtifactSpec {
    kind: String,
    title: String,
    metadata: Value,
    body: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeStageOutcome {
    outcome: String,
    /// Explicitly cited, already-linked TaskHistory Artifact identities.
    #[serde(default)]
    artifact_ids: Vec<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    cancellation_reason_key: Option<String>,
}

#[derive(Debug, Error)]
enum FakeRunSpecError {
    #[error("unsupported M0 RunSpec version {received}")]
    UnsupportedVersion { received: u32 },
    #[error("RunSpec is not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error("artifact {index} metadata is not a JSON object")]
    MetadataMustBeObject { index: usize },
    #[error("artifact {index} metadata is invalid: {source}")]
    InvalidArtifactMetadata {
        index: usize,
        source: ArtifactMetadataError,
    },
    #[error("artifact {index} body cannot be serialized as JSON")]
    ArtifactBodyCannotSerialize { index: usize },
    #[error("artifact {index} body exceeds the M0 inline evidence limit")]
    ArtifactBodyTooLarge { index: usize },
    #[error("stage outcome must not be blank")]
    BlankOutcome,
    #[error("stage outcome must not cite a TaskHistory Artifact more than once")]
    DuplicateTaskHistoryArtifactIds,
}

impl FakeRunSpecError {
    fn reason_code(&self) -> &'static str {
        match self {
            Self::UnsupportedVersion { .. } => "unsupported_run_spec_version",
            Self::Json(_) => "invalid_run_spec_json",
            Self::MetadataMustBeObject { .. } => "invalid_run_spec_metadata",
            Self::InvalidArtifactMetadata { .. } => "invalid_run_spec_metadata",
            Self::ArtifactBodyCannotSerialize { .. } | Self::ArtifactBodyTooLarge { .. } => {
                "invalid_run_spec_artifact_body"
            }
            Self::BlankOutcome => "invalid_run_spec_outcome",
            Self::DuplicateTaskHistoryArtifactIds => "duplicate_run_spec_task_history_artifact_ids",
        }
    }
}

fn deserialize_delay<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let milliseconds = Option::<u64>::deserialize(deserializer)?.unwrap_or_default();
    if milliseconds > 60_000 {
        return Err(serde::de::Error::custom("delay_ms exceeds 60000"));
    }
    Ok(Duration::from_millis(milliseconds))
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
