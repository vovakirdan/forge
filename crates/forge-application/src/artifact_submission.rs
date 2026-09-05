use std::collections::BTreeSet;

use forge_domain::{
    Actor, ArtifactBody, ArtifactId, CancellationReasonId, NewArtifact, OutcomeKey, ProjectId,
    StageId, Timestamp, WaitConditionId,
};
use forge_protocol::wire::{
    CommandName, MAX_INLINE_ARTIFACT_BODY_BYTES, validate_artifact_metadata,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    ApplicationError,
    command::parse_uuid_v7,
    payload::{artifact_kind, bounded_len, domain_error, outcome_key, stage_id},
};

const MAX_EXTERNAL_ARTIFACTS: usize = 32;

/// Structured evidence supplied by a human/external outcome command.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInput {
    /// Stable Artifact kind.
    pub kind: String,
    /// Human-readable title.
    pub title: String,
    /// Structured metadata object.
    #[serde(default = "empty_object")]
    pub metadata: Map<String, Value>,
    /// M0 inline JSON body.
    pub body: Value,
}

impl ArtifactInput {
    /// Rejects metadata and inline-body input that exceeds the public M0
    /// evidence limits before Core can create an Artifact.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidPayload`] when metadata violates the
    /// shared protocol policy or the serialized body exceeds its byte limit.
    pub fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        validate_artifact_metadata(&self.metadata).map_err(|error| {
            ApplicationError::InvalidPayload {
                command: CommandName::SubmitExternalStageOutcome,
                reason: format!("artifact metadata {error}"),
            }
        })?;
        let serialized =
            serde_json::to_vec(&self.body).map_err(|error| ApplicationError::InvalidPayload {
                command: CommandName::SubmitExternalStageOutcome,
                reason: format!("cannot serialize artifact body: {error}"),
            })?;
        if serialized.len() > MAX_INLINE_ARTIFACT_BODY_BYTES {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::SubmitExternalStageOutcome,
                reason: format!(
                    "artifact body exceeds {MAX_INLINE_ARTIFACT_BODY_BYTES} serialized bytes"
                ),
            });
        }
        Ok(())
    }

    /// Converts the input to immutable Artifact construction data.
    ///
    /// # Errors
    ///
    /// Returns the same public-contract refusal as
    /// [`Self::validate_public_contract`] before creating domain input.
    pub fn to_new_artifact(
        &self,
        project_id: ProjectId,
        created_by: Actor,
        created_at: Timestamp,
    ) -> Result<NewArtifact, ApplicationError> {
        self.validate_public_contract()?;
        Ok(NewArtifact {
            project_id,
            kind: artifact_kind("artifacts.kind", &self.kind)?,
            title: self.title.clone(),
            body: ArtifactBody::inline_json(self.body.clone()),
            metadata: Value::Object(self.metadata.clone()),
            created_by,
            created_at,
        })
    }
}

/// Human/external resolution of one waiting Pipeline stage.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalStageOutcomeCommand {
    /// Target Task.
    pub task_id: String,
    /// Expected Task revision.
    pub expected_task_revision: u64,
    /// Exact active stage identity.
    pub stage_id: String,
    /// Outcome declared in the pinned Pipeline graph.
    pub outcome: String,
    /// Exact wait-condition identity held by this manual stage.
    pub wait_condition_id: String,
    /// Optional catalog reason used only by cancellation transitions.
    #[serde(default)]
    pub cancellation_reason_key: Option<String>,
    /// Evidence to attach before outcome validation.
    #[serde(default)]
    pub artifacts: Vec<ArtifactInput>,
    /// Already-linked evidence explicitly cited by this outcome.
    #[serde(default)]
    pub outcome_artifact_ids: Vec<String>,
}

impl ExternalStageOutcomeCommand {
    /// Parses the target Task and active wait identity from the public body.
    pub fn identifiers(&self) -> Result<(forge_domain::TaskId, WaitConditionId), ApplicationError> {
        Ok((
            parse_uuid_v7("task_id", &self.task_id)?,
            parse_uuid_v7("wait_condition_id", &self.wait_condition_id)?,
        ))
    }

    /// Converts an optional cancellation key to its domain value.
    pub fn cancellation_reason_id(&self) -> Result<Option<CancellationReasonId>, ApplicationError> {
        self.cancellation_reason_key
            .as_ref()
            .map(|value| CancellationReasonId::new(value.clone()).map_err(domain_error))
            .transpose()
    }

    /// Parses the stage that the caller believes is currently active.
    pub fn stage_id(&self) -> Result<StageId, ApplicationError> {
        stage_id("stage_id", &self.stage_id)
    }

    /// Parses the declared Pipeline outcome key.
    pub fn outcome(&self) -> Result<OutcomeKey, ApplicationError> {
        outcome_key("outcome", &self.outcome)
    }

    /// Parses explicitly cited, previously linked Artifact identities.
    pub fn outcome_artifact_ids(&self) -> Result<BTreeSet<ArtifactId>, ApplicationError> {
        if self.outcome_artifact_ids.len() > 128 {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::SubmitExternalStageOutcome,
                reason: "outcome_artifact_ids must contain at most 128 identities".to_owned(),
            });
        }
        let artifact_ids = self
            .outcome_artifact_ids
            .iter()
            .map(|value| parse_uuid_v7("outcome_artifact_ids", value))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if artifact_ids.len() != self.outcome_artifact_ids.len() {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::SubmitExternalStageOutcome,
                reason: "outcome_artifact_ids must not contain duplicates".to_owned(),
            });
        }
        Ok(artifact_ids)
    }

    /// Rejects a direct typed call that would violate any public payload cap.
    pub fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        bounded_len(
            CommandName::SubmitExternalStageOutcome,
            "artifacts",
            self.artifacts.len(),
            0,
            MAX_EXTERNAL_ARTIFACTS,
        )?;
        for artifact in &self.artifacts {
            artifact.validate_public_contract()?;
        }
        Ok(())
    }
}

fn empty_object() -> Map<String, Value> {
    Map::new()
}
