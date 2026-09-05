use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{DomainError, ids::validate_stable_key};

/// Stable Project-defined identifier for a scheduler priority level.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PriorityLevelId(String);

impl PriorityLevelId {
    /// Validates a stable priority-level key.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIdentifier`] for a malformed key.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_stable_key("priority_level_id", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// One Project-owned level in a scheduler priority scheme.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PriorityLevel {
    id: PriorityLevelId,
    display_name: String,
    rank: i32,
    retired: bool,
}

impl PriorityLevel {
    /// Creates an active priority level.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for a blank display name.
    pub fn new(
        id: PriorityLevelId,
        display_name: impl Into<String>,
        rank: i32,
    ) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() || display_name.chars().count() > 128 {
            return Err(DomainError::InvalidValue {
                field: "priority_level.display_name",
                reason: "must be non-blank and at most 128 characters".to_owned(),
            });
        }
        Ok(Self {
            id,
            display_name,
            rank,
            retired: false,
        })
    }

    /// Returns the stable level identifier.
    #[must_use]
    pub fn id(&self) -> &PriorityLevelId {
        &self.id
    }

    /// Returns the human-facing level label.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the scheduler comparison rank.
    #[must_use]
    pub const fn rank(&self) -> i32 {
        self.rank
    }

    /// Returns whether new Tasks may choose this level.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.retired
    }

    /// Retires this level without changing historical Task records.
    pub fn retire(&mut self) {
        self.retired = true;
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        let mut restored = Self::new(self.id.clone(), self.display_name.clone(), self.rank)?;
        if self.retired {
            restored.retire();
        }
        if restored == *self {
            Ok(())
        } else {
            Err(DomainError::InvalidValue {
                field: "priority_level.snapshot",
                reason: "is not canonical".to_owned(),
            })
        }
    }
}

/// Project-configurable priority scheme used by deterministic scheduling.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PriorityScheme {
    levels: BTreeMap<PriorityLevelId, PriorityLevel>,
    default_level_id: PriorityLevelId,
}

impl PriorityScheme {
    /// Creates a priority scheme from a non-empty, unique set of levels.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] if levels are missing, duplicate,
    /// or the default does not name an active level.
    pub fn new(
        levels: impl IntoIterator<Item = PriorityLevel>,
        default_level_id: PriorityLevelId,
    ) -> Result<Self, DomainError> {
        let mut values = BTreeMap::new();
        for level in levels {
            let id = level.id.clone();
            if values.insert(id.clone(), level).is_some() {
                return Err(DomainError::InvalidValue {
                    field: "priority_scheme.levels",
                    reason: format!("duplicate level: {}", id.as_str()),
                });
            }
        }
        let Some(default_level) = values.get(&default_level_id) else {
            return Err(DomainError::InvalidValue {
                field: "priority_scheme.default_level_id",
                reason: "must reference a defined level".to_owned(),
            });
        };
        if default_level.retired {
            return Err(DomainError::InvalidValue {
                field: "priority_scheme.default_level_id",
                reason: "must reference an active level".to_owned(),
            });
        }
        Ok(Self {
            levels: values,
            default_level_id,
        })
    }

    /// Builds Forge's conventional three-level default scheme.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError`] only if an invariant in the default model is
    /// changed in the future.
    pub fn default_three_levels() -> Result<Self, DomainError> {
        let low = PriorityLevel::new(PriorityLevelId::new("low")?, "Low", 10)?;
        let normal = PriorityLevel::new(PriorityLevelId::new("normal")?, "Normal", 50)?;
        let high = PriorityLevel::new(PriorityLevelId::new("high")?, "High", 100)?;
        Self::new([low, normal, high], PriorityLevelId::new("normal")?)
    }

    /// Returns the level assigned to new Tasks by default.
    #[must_use]
    pub fn default_level_id(&self) -> &PriorityLevelId {
        &self.default_level_id
    }

    /// Returns a level by stable identifier.
    #[must_use]
    pub fn get(&self, level_id: &PriorityLevelId) -> Option<&PriorityLevel> {
        self.levels.get(level_id)
    }

    /// Validates that a Task may select this active priority level.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] when the level is unknown or retired.
    pub fn validate_active_level(&self, level_id: &PriorityLevelId) -> Result<(), DomainError> {
        let Some(level) = self.levels.get(level_id) else {
            return Err(DomainError::InvalidValue {
                field: "task.priority_level_id",
                reason: format!("unknown level: {}", level_id.as_str()),
            });
        };
        if level.retired {
            return Err(DomainError::InvalidValue {
                field: "task.priority_level_id",
                reason: format!("retired level: {}", level_id.as_str()),
            });
        }
        Ok(())
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.default_level_id.validate()?;
        for level in self.levels.values() {
            level.validate_snapshot()?;
        }
        Self::new(self.levels.values().cloned(), self.default_level_id.clone()).map(|_| ())
    }
}

/// Stable Project-defined identifier for a cancellation reason.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CancellationReasonId(String);

impl CancellationReasonId {
    /// The minimum catalog reason that every Project must retain.
    pub const UNSPECIFIED: &'static str = "unspecified";

    /// Validates a stable cancellation-reason key.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIdentifier`] for a malformed key.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_stable_key("cancellation_reason_id", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable catalog key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// One Project-defined cancellation reason retained for durable filtering.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancellationReason {
    id: CancellationReasonId,
    display_name: String,
    retired: bool,
}

impl CancellationReason {
    /// Creates an active catalog reason.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for a blank display name.
    pub fn new(
        id: CancellationReasonId,
        display_name: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() || display_name.chars().count() > 200 {
            return Err(DomainError::InvalidValue {
                field: "cancellation_reason.display_name",
                reason: "must be non-blank and at most 200 characters".to_owned(),
            });
        }
        Ok(Self {
            id,
            display_name,
            retired: false,
        })
    }

    /// Creates the mandatory fallback catalog reason.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError`] only if the built-in stable key becomes invalid.
    pub fn unspecified() -> Result<Self, DomainError> {
        Self::new(
            CancellationReasonId::new(CancellationReasonId::UNSPECIFIED)?,
            "Unspecified",
        )
    }

    /// Returns the stable catalog key.
    #[must_use]
    pub fn id(&self) -> &CancellationReasonId {
        &self.id
    }

    /// Returns the human-facing catalog label.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns whether new cancellation commands may choose this reason.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.retired
    }

    /// Retires this reason while retaining existing cancellation history.
    pub fn retire(&mut self) {
        self.retired = true;
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        let mut restored = Self::new(self.id.clone(), self.display_name.clone())?;
        if self.retired {
            restored.retire();
        }
        if restored == *self {
            Ok(())
        } else {
            Err(DomainError::InvalidValue {
                field: "cancellation_reason.snapshot",
                reason: "is not canonical".to_owned(),
            })
        }
    }
}

/// Project-owned catalog that validates every terminal cancellation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancellationReasonCatalog {
    reasons: BTreeMap<CancellationReasonId, CancellationReason>,
}

impl CancellationReasonCatalog {
    /// Creates a catalog that includes an active `unspecified` reason.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for duplicate entries or an absent
    /// or retired fallback reason.
    pub fn new(reasons: impl IntoIterator<Item = CancellationReason>) -> Result<Self, DomainError> {
        let mut values = BTreeMap::new();
        for reason in reasons {
            let id = reason.id.clone();
            if values.insert(id.clone(), reason).is_some() {
                return Err(DomainError::InvalidValue {
                    field: "cancellation_reason_catalog",
                    reason: format!("duplicate reason: {}", id.as_str()),
                });
            }
        }
        let unspecified_id = CancellationReasonId::new(CancellationReasonId::UNSPECIFIED)?;
        let Some(unspecified) = values.get(&unspecified_id) else {
            return Err(DomainError::InvalidValue {
                field: "cancellation_reason_catalog",
                reason: "must include an unspecified reason".to_owned(),
            });
        };
        if unspecified.retired {
            return Err(DomainError::InvalidValue {
                field: "cancellation_reason_catalog",
                reason: "unspecified reason must remain active".to_owned(),
            });
        }
        Ok(Self { reasons: values })
    }

    /// Returns the active reason with `reason_id`, if available.
    #[must_use]
    pub fn active(&self, reason_id: &CancellationReasonId) -> Option<&CancellationReason> {
        self.reasons.get(reason_id).filter(|reason| !reason.retired)
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        for reason in self.reasons.values() {
            reason.validate_snapshot()?;
        }
        Self::new(self.reasons.values().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{CancellationReason, CancellationReasonCatalog, PriorityLevelId, PriorityScheme};

    #[test]
    fn three_level_scheme_uses_normal_as_its_default() {
        let scheme = PriorityScheme::default_three_levels().expect("valid built-in scheme");

        assert_eq!(scheme.default_level_id().as_str(), "normal");
    }

    #[test]
    fn cancellation_catalog_requires_unspecified_reason() {
        let other = CancellationReason::new(
            super::CancellationReasonId::new("duplicate").expect("valid reason id"),
            "Duplicate",
        )
        .expect("valid reason");

        let catalog = CancellationReasonCatalog::new([other]);

        assert!(catalog.is_err());
    }

    #[test]
    fn scheme_rejects_an_unknown_task_priority() {
        let scheme = PriorityScheme::default_three_levels().expect("valid built-in scheme");
        let unknown = PriorityLevelId::new("urgent").expect("valid level id");

        let result = scheme.validate_active_level(&unknown);

        assert!(result.is_err());
    }
}
