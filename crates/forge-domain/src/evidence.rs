//! Scoped technical evidence. These receipts never imply Artifact acceptance.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, ProjectId, TaskId, Timestamp};

/// Project, Task and attempt that own one technical evidence body.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceScope {
    pub project_id: ProjectId,
    /// Present only for TaskStage-owned evidence. Taskless bytes are scoped by Run.
    pub task_id: Option<TaskId>,
    pub run_id: Uuid,
}

impl EvidenceScope {
    /// Validates canonical identities before filesystem or object-store access.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.project_id.validate_v7("evidence.project_id")?;
        if let Some(task_id) = self.task_id {
            task_id.validate_v7("evidence.task_id")?;
        }
        validate_uuid(self.run_id, "evidence.run_id")
    }
}

/// Source of technical bytes, separate from canonical Task Artifacts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStream {
    Stdout,
    Stderr,
    Diagnostic,
}

/// The sole body location currently established by a receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EvidenceLocation {
    /// Bytes have been fsynced locally but have no confirmed remote copy.
    PendingUpload,
    /// Object-store put succeeded and its immutable scoped key is recorded.
    Stored { object_key: String },
}

/// Immutable metadata for a redacted evidence chunk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct EvidenceObject(EvidenceObjectInput);

/// Validated input for an immutable technical evidence receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceObjectInput {
    pub id: Uuid,
    pub scope: EvidenceScope,
    pub stream: EvidenceStream,
    pub sequence: u64,
    /// Lowercase SHA-256 hex digest of the redacted stored bytes.
    pub sha256: String,
    pub size_bytes: u64,
    /// Stable policy/version identifier, never secret values themselves.
    pub redaction_policy_reference: String,
    pub location: EvidenceLocation,
    pub created_at: Timestamp,
}

impl EvidenceObject {
    /// Checks immutable object identity, digest and scoped object location.
    pub fn new(input: EvidenceObjectInput) -> Result<Self, DomainError> {
        validate_uuid(input.id, "evidence.id")?;
        input.scope.validate()?;
        if input.sequence == 0 || input.size_bytes == 0 {
            return Err(invalid("evidence", "sequence and size must be positive"));
        }
        if input.sha256.len() != 64
            || !input
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "evidence.sha256",
                "must be a lowercase SHA-256 hex digest",
            ));
        }
        validate_reference(
            &input.redaction_policy_reference,
            "evidence.redaction_policy",
        )?;
        let result = Self(input);
        if let EvidenceLocation::Stored { object_key } = &result.0.location
            && object_key != &result.object_key()
        {
            return Err(invalid(
                "evidence.object_key",
                "does not match its immutable scope",
            ));
        }
        Ok(result)
    }

    /// Returns the frozen receipt without mutable access.
    pub fn data(&self) -> &EvidenceObjectInput {
        &self.0
    }

    /// Stable collision-resistant object key derived exclusively from canonical metadata.
    pub fn object_key(&self) -> String {
        let scope = self.0.scope;
        match scope.task_id {
            Some(task_id) => format!(
                "evidence/{}/{}/{}/{}/{}",
                scope.project_id, task_id, scope.run_id, self.0.id, self.0.sha256
            ),
            None => format!(
                "evidence/{}/runs/{}/{}/{}",
                scope.project_id, scope.run_id, self.0.id, self.0.sha256
            ),
        }
    }

    /// Returns a new receipt after the adapter has confirmed durable upload.
    /// The original pending receipt remains unchanged for the audit history.
    pub fn stored(&self) -> Result<Self, DomainError> {
        let mut input = self.0.clone();
        input.location = EvidenceLocation::Stored {
            object_key: self.object_key(),
        };
        Self::new(input)
    }
}

impl<'de> Deserialize<'de> for EvidenceObject {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(EvidenceObjectInput::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

pub(crate) fn validate_uuid(id: Uuid, field: &'static str) -> Result<(), DomainError> {
    if id.get_version_num() == 7 {
        Ok(())
    } else {
        Err(invalid(field, "must be a UUIDv7"))
    }
}

pub(crate) fn validate_reference(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
    {
        Err(invalid(field, "must be a bounded non-secret reference"))
    } else {
        Ok(())
    }
}

pub(crate) fn invalid(field: &'static str, reason: &str) -> DomainError {
    DomainError::InvalidValue {
        field,
        reason: reason.to_owned(),
    }
}
