use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Actor, ArtifactId, DomainError, EmployeeId, ProjectId, StageId, StageVisit, Timestamp,
    ids::validate_stable_key,
};

/// A Project-defined, stable type of evidence attached to a Task.
///
/// The core recognizes a small set of conventional names but does not make the
/// vocabulary closed: a Pipeline may require a Project-specific artifact kind.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactKind(String);

/// Maximum serialized byte length of an M0 inline JSON Artifact body.
///
/// This is a storage-safety boundary, not a quality or semantic check. Later
/// object storage can represent larger evidence through [`ArtifactBody::ObjectReference`].
pub const MAX_INLINE_JSON_BODY_BYTES: usize = 256 * 1024;

impl ArtifactKind {
    /// Canonical kind for an analysis outcome.
    pub const ANALYSIS_RESULT: &'static str = "analysis_result";
    /// Canonical kind for an authoritative human or resolver decision.
    pub const DECISION_RECORD: &'static str = "decision_record";
    /// Canonical kind for a delivery implementation result.
    pub const CHANGE_SET: &'static str = "change_set";
    /// Canonical kind for a plan.
    pub const PLAN: &'static str = "plan";
    /// Canonical kind for a review report.
    pub const REVIEW_RESULT: &'static str = "review_result";
    /// Canonical kind for a verification report.
    pub const VERIFICATION_REPORT: &'static str = "verification_report";

    /// Validates a stable artifact-kind key.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIdentifier`] when the key is malformed.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_stable_key("artifact_kind", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable artifact-kind key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }

    /// Returns whether this kind is the required result of an analysis Task.
    #[must_use]
    pub fn is_analysis_result(&self) -> bool {
        self.0 == Self::ANALYSIS_RESULT
    }
}

/// Canonical Artifact content. M0 persists only [`ArtifactBody::InlineJson`];
/// object references reserve the domain shape required by later object storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "storage", rename_all = "snake_case")]
pub enum ArtifactBody {
    /// Structured body stored with canonical Artifact metadata.
    InlineJson {
        /// Arbitrary structured evidence. Its semantics are Pipeline-owned.
        value: Value,
    },
    /// Immutable pointer to a body held in object storage.
    ObjectReference {
        /// Opaque immutable object key.
        object_key: String,
        /// Optional media type known at submission time.
        media_type: Option<String>,
        /// Optional content digest, formatted by the storage adapter.
        content_digest: Option<String>,
    },
}

impl ArtifactBody {
    /// Constructs an inline JSON Artifact body.
    #[must_use]
    pub const fn inline_json(value: Value) -> Self {
        Self::InlineJson { value }
    }

    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::InlineJson { value } => {
                let serialized =
                    serde_json::to_vec(value).map_err(|error| DomainError::InvalidValue {
                        field: "artifact.body",
                        reason: format!("cannot serialize inline JSON: {error}"),
                    })?;
                if serialized.len() > MAX_INLINE_JSON_BODY_BYTES {
                    return Err(DomainError::InvalidValue {
                        field: "artifact.body",
                        reason: format!(
                            "serialized inline JSON must not exceed {MAX_INLINE_JSON_BODY_BYTES} bytes"
                        ),
                    });
                }
                Ok(())
            }
            Self::ObjectReference {
                object_key,
                media_type: _,
                content_digest: _,
            } if object_key.trim().is_empty() => Err(DomainError::InvalidValue {
                field: "artifact.object_key",
                reason: "must not be blank".to_owned(),
            }),
            Self::ObjectReference { .. } => Ok(()),
        }
    }
}

/// Immutable evidence produced during work or supplied by a human.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    id: ArtifactId,
    project_id: ProjectId,
    kind: ArtifactKind,
    title: String,
    body: ArtifactBody,
    metadata: Value,
    created_by: Actor,
    created_at: Timestamp,
}

/// Input used to create one immutable [`Artifact`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewArtifact {
    /// Project that owns the Artifact.
    pub project_id: ProjectId,
    /// Stable Project-defined evidence kind.
    pub kind: ArtifactKind,
    /// Human-readable title.
    pub title: String,
    /// Immutable evidence body.
    pub body: ArtifactBody,
    /// Structured metadata, which must be a JSON object.
    pub metadata: Value,
    /// Actor submitting the evidence.
    pub created_by: Actor,
    /// Submission timestamp.
    pub created_at: Timestamp,
}

impl Artifact {
    /// Constructs immutable, structurally valid evidence.
    ///
    /// `metadata` must be a JSON object. Core and Pipeline validate only the
    /// declared structure; they do not infer the truth of its contents.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for a blank title, non-object
    /// metadata, or malformed body reference.
    pub fn new(id: ArtifactId, input: NewArtifact) -> Result<Self, DomainError> {
        if input.title.trim().is_empty() || input.title.chars().count() > 240 {
            return Err(DomainError::InvalidValue {
                field: "artifact.title",
                reason: "must be non-blank and at most 240 characters".to_owned(),
            });
        }
        if !input.metadata.is_object() {
            return Err(DomainError::InvalidValue {
                field: "artifact.metadata",
                reason: "must be a JSON object".to_owned(),
            });
        }
        input.body.validate()?;

        Ok(Self {
            id,
            project_id: input.project_id,
            kind: input.kind,
            title: input.title,
            body: input.body,
            metadata: input.metadata,
            created_by: input.created_by,
            created_at: input.created_at,
        })
    }

    /// Returns the immutable Artifact ID.
    #[must_use]
    pub const fn id(&self) -> ArtifactId {
        self.id
    }

    /// Returns the Project that owns the Artifact.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the Artifact type.
    #[must_use]
    pub fn kind(&self) -> &ArtifactKind {
        &self.kind
    }

    /// Returns its human-readable title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the immutable evidence body.
    #[must_use]
    pub fn body(&self) -> &ArtifactBody {
        &self.body
    }

    /// Returns structured metadata without interpreting it.
    #[must_use]
    pub fn metadata(&self) -> &Value {
        &self.metadata
    }

    /// Returns the actor that submitted the Artifact.
    #[must_use]
    pub const fn created_by(&self) -> Actor {
        self.created_by
    }

    /// Returns when the Artifact was submitted.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Revalidates a deserialized immutable Artifact before storage exposes it.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("artifact.id")?;
        self.project_id.validate_v7("artifact.project_id")?;
        self.kind.validate()?;
        self.created_by.validate_snapshot()?;
        Self::new(
            self.id,
            NewArtifact {
                project_id: self.project_id,
                kind: self.kind.clone(),
                title: self.title.clone(),
                body: self.body.clone(),
                metadata: self.metadata.clone(),
                created_by: self.created_by,
                created_at: self.created_at,
            },
        )
        .map(|_| ())
    }
}

/// Scope from which an Artifact was attached to a Task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactProducer {
    /// Evidence submitted while an Employee stage was executing.
    EmployeeRun,
    /// Evidence submitted while resolving an escalation or other question.
    ResolutionAssignment,
    /// Evidence attached directly by a human actor.
    Human,
    /// Evidence generated by a deterministic system stage.
    System,
}

/// Ordered Task-local reference to immutable evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactLink {
    artifact_id: ArtifactId,
    kind: ArtifactKind,
    producer: ArtifactProducer,
    source_stage_id: Option<StageId>,
    source_stage_visit: Option<StageVisit>,
    submitted_by: Actor,
    submitted_at: Timestamp,
    employee_id: Option<EmployeeId>,
}

impl ArtifactLink {
    /// Creates a Task-local link from a submitted Artifact.
    #[must_use]
    pub(crate) fn from_artifact(
        artifact: &Artifact,
        producer: ArtifactProducer,
        source_stage_id: Option<StageId>,
        source_stage_visit: Option<StageVisit>,
        submitted_by: Actor,
        submitted_at: Timestamp,
        employee_id: Option<EmployeeId>,
    ) -> Self {
        Self {
            artifact_id: artifact.id(),
            kind: artifact.kind().clone(),
            producer,
            source_stage_id,
            source_stage_visit,
            submitted_by,
            submitted_at,
            employee_id,
        }
    }

    /// Returns the linked Artifact identity.
    #[must_use]
    pub const fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    /// Returns its kind as captured at attachment time.
    #[must_use]
    pub fn kind(&self) -> &ArtifactKind {
        &self.kind
    }

    /// Returns the producer scope.
    #[must_use]
    pub const fn producer(&self) -> ArtifactProducer {
        self.producer
    }

    /// Returns the Pipeline stage that produced the Artifact, if known.
    #[must_use]
    pub fn source_stage_id(&self) -> Option<&StageId> {
        self.source_stage_id.as_ref()
    }

    /// Returns the Task-local stage visit that produced the Artifact, if known.
    #[must_use]
    pub const fn source_stage_visit(&self) -> Option<StageVisit> {
        self.source_stage_visit
    }

    /// Returns the actor that attached the Artifact.
    #[must_use]
    pub const fn submitted_by(&self) -> Actor {
        self.submitted_by
    }

    /// Returns when it was linked to the Task.
    #[must_use]
    pub const fn submitted_at(&self) -> Timestamp {
        self.submitted_at
    }

    /// Returns the Employee associated with the submission, if any.
    #[must_use]
    pub const fn employee_id(&self) -> Option<EmployeeId> {
        self.employee_id
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.artifact_id.validate_v7("artifact_link.artifact_id")?;
        self.kind.validate()?;
        self.submitted_by.validate_snapshot()?;
        if let Some(stage_id) = &self.source_stage_id {
            stage_id.validate()?;
        }
        if self.source_stage_id.is_some() != self.source_stage_visit.is_some() {
            return Err(DomainError::InvalidValue {
                field: "artifact_link.source_stage",
                reason: "stage ID and visit must either both be present or both be absent"
                    .to_owned(),
            });
        }
        if let Some(employee_id) = self.employee_id {
            employee_id.validate_v7("artifact_link.employee_id")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Artifact, ArtifactBody, ArtifactKind, NewArtifact};
    use crate::{Actor, ActorId, ArtifactId, ProjectId, Timestamp};

    #[test]
    fn artifact_rejects_non_object_metadata() {
        let result = Artifact::new(
            ArtifactId::new(),
            NewArtifact {
                project_id: ProjectId::new(),
                kind: ArtifactKind::new(ArtifactKind::PLAN).expect("valid kind"),
                title: "Plan".to_owned(),
                body: ArtifactBody::inline_json(json!({"body": "content"})),
                metadata: json!(["not", "metadata"]),
                created_by: Actor::human(ActorId::new()),
                created_at: Timestamp::now_utc(),
            },
        );

        assert!(result.is_err());
    }
}
