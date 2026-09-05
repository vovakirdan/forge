use std::{fmt, num::NonZeroU64, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::DomainError;

macro_rules! uuid_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a time-sortable UUIDv7 identity.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Returns the underlying UUID for storage and transport adapters.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }

            pub(crate) fn validate_v7(self, field: &'static str) -> Result<(), DomainError> {
                if self.0.get_version_num() == 7 {
                    Ok(())
                } else {
                    Err(DomainError::InvalidValue {
                        field,
                        reason: "must be a UUIDv7".to_owned(),
                    })
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}

uuid_id!(ProjectId, "Identity of a Forge Project.");
uuid_id!(TaskId, "Global identity of a Task.");
uuid_id!(EmployeeId, "Global identity of an Employee.");
uuid_id!(ArtifactId, "Global identity of an Artifact.");
uuid_id!(EventId, "Global identity of an immutable domain Event.");
uuid_id!(
    ActorId,
    "Global identity of a human, service, or manager actor."
);
uuid_id!(
    CommandId,
    "Idempotency identity of a named command invocation."
);
uuid_id!(PipelineId, "Global identity of a Project Pipeline.");
uuid_id!(
    PipelineVersionId,
    "Global identity of an immutable Pipeline version."
);
uuid_id!(
    WaitConditionId,
    "Global identity of a typed Task wait condition."
);

/// A Project-local, monotonically allocated ordinal.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectLocalSequence(NonZeroU64);

impl ProjectLocalSequence {
    /// Builds a non-zero Project-local ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] when `value` is zero.
    pub fn new(value: u64) -> Result<Self, DomainError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or_else(|| DomainError::InvalidValue {
                field: "project_local_sequence",
                reason: "must be greater than zero".to_owned(),
            })
    }

    /// Returns the ordinal as a primitive integer.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for ProjectLocalSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Human-readable Project-local Task key such as `TASK-001`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskKey(ProjectLocalSequence);

impl TaskKey {
    /// Creates a Task key from its Project-local sequence.
    #[must_use]
    pub const fn new(sequence: ProjectLocalSequence) -> Self {
        Self(sequence)
    }

    /// Returns the Project-local sequence backing this key.
    #[must_use]
    pub const fn sequence(self) -> ProjectLocalSequence {
        self.0
    }
}

impl fmt::Display for TaskKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "TASK-{:03}", self.0.get())
    }
}

impl FromStr for TaskKey {
    type Err = TaskKeyParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(sequence) = value.strip_prefix("TASK-") else {
            return Err(TaskKeyParseError::MissingPrefix);
        };

        if sequence.is_empty() || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(TaskKeyParseError::InvalidSequence);
        }

        let sequence = sequence
            .parse::<u64>()
            .map_err(|_| TaskKeyParseError::InvalidSequence)?;
        let sequence =
            ProjectLocalSequence::new(sequence).map_err(|_| TaskKeyParseError::InvalidSequence)?;
        Ok(Self::new(sequence))
    }
}

/// Error returned while parsing a human-readable [`TaskKey`].
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum TaskKeyParseError {
    /// The key does not start with `TASK-`.
    #[error("task key must start with TASK-")]
    MissingPrefix,
    /// The numeric suffix is absent, zero, or malformed.
    #[error("task key sequence must be a positive decimal integer")]
    InvalidSequence,
}

/// Monotonic revision of the mutable Task projection.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskRevision(NonZeroU64);

impl TaskRevision {
    /// First revision assigned to a newly created Task.
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    /// Returns the revision as a primitive integer.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Returns the next revision.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::RevisionOverflow`] if the revision reaches `u64::MAX`.
    pub fn next(self) -> Result<Self, DomainError> {
        let value = self
            .get()
            .checked_add(1)
            .ok_or(DomainError::RevisionOverflow)?;
        Self::from_non_zero(value)
    }

    pub(crate) fn from_non_zero(value: u64) -> Result<Self, DomainError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or_else(|| DomainError::InvalidValue {
                field: "task_revision",
                reason: "must be greater than zero".to_owned(),
            })
    }
}

/// Monotonic Task-local identity of one entry into a Pipeline stage.
///
/// Stage IDs may recur in a graph. A visit identity distinguishes evidence
/// produced during a previous pass through the same stage from evidence
/// produced during the active pass.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StageVisit(NonZeroU64);

impl StageVisit {
    /// First stage visit assigned when an approved Task enters its entry stage.
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    /// Returns the underlying Task-local sequence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Allocates the next distinct stage visit for one Task.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] on the theoretical sequence
    /// overflow boundary.
    pub fn next(self) -> Result<Self, DomainError> {
        let value = self
            .get()
            .checked_add(1)
            .ok_or_else(|| DomainError::InvalidValue {
                field: "task.stage_visit",
                reason: "cannot exceed u64::MAX".to_owned(),
            })?;
        NonZeroU64::new(value)
            .map(Self)
            .ok_or_else(|| DomainError::InvalidValue {
                field: "task.stage_visit",
                reason: "must be greater than zero".to_owned(),
            })
    }
}

/// Stable Pipeline-stage identifier, scoped by a Pipeline version.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StageId(String);

impl StageId {
    /// Validates and creates a stable stage identifier.
    ///
    /// Stage keys are ASCII lowercase identifiers so they remain durable in
    /// Pipeline versions, APIs, and artifact links.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIdentifier`] for a malformed key.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_stable_key("stage_id", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable identifier string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

impl fmt::Display for StageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A wall-clock timestamp normalized to UTC for domain history.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(OffsetDateTime);

impl Timestamp {
    /// Captures the current UTC time.
    #[must_use]
    pub fn now_utc() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    /// Wraps an existing UTC-capable timestamp, normalizing its offset.
    #[must_use]
    pub fn from_offset_date_time(value: OffsetDateTime) -> Self {
        Self(value.to_offset(time::UtcOffset::UTC))
    }

    /// Returns the UTC timestamp for adapters that need to serialize it.
    #[must_use]
    pub const fn as_offset_date_time(self) -> OffsetDateTime {
        self.0
    }
}

/// Validates a stable lower-snake-style identifier shared by core value objects.
pub(crate) fn validate_stable_key(field: &'static str, value: &str) -> Result<(), DomainError> {
    let valid_length = (1..=64).contains(&value.len());
    let valid_first = value.as_bytes().first().is_some_and(u8::is_ascii_lowercase);
    let valid_rest = value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');

    if valid_length && valid_first && valid_rest {
        Ok(())
    } else {
        Err(DomainError::InvalidIdentifier {
            field,
            reason: "must be 1..=64 lowercase ASCII letters, digits, or underscores and start with a letter".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{ProjectLocalSequence, TaskKey};

    #[test]
    fn task_key_formats_with_a_stable_prefix_and_padding() {
        let sequence = ProjectLocalSequence::new(1).expect("positive sequence");

        assert_eq!(TaskKey::new(sequence).to_string(), "TASK-001");
    }

    #[test]
    fn task_key_rejects_zero_sequence() {
        let parsed = TaskKey::from_str("TASK-0");

        assert!(parsed.is_err());
    }

    #[test]
    fn stage_id_rejects_presentation_labels() {
        let stage = super::StageId::new("Needs Review");

        assert!(stage.is_err());
    }
}
