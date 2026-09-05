use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{DomainError, TaskProperties, TaskPropertySchema};

/// Work category that determines a Task's immediate outcome contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// A targeted change to code, configuration, documentation, tests, or another Project asset.
    Delivery,
    /// Investigation that produces verified knowledge and a recommendation.
    Analysis,
}

/// Origin of Task intent, retained independently of its lifecycle and Pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    /// Created explicitly by a human or manager.
    Human,
    /// Created from a collected external report.
    ExternalReport,
    /// Promoted from an observed Finding.
    PromotedFinding,
    /// Created from an approved plan.
    ApprovedPlan,
}

/// Boundaries and constraints that give Task intent a durable shape.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskScope {
    /// Work explicitly included in the Task.
    pub in_scope: Vec<String>,
    /// Work explicitly excluded from the Task.
    pub out_of_scope: Vec<String>,
    /// Constraints that a successful outcome must respect.
    pub constraints: Vec<String>,
}

/// Input used to validate and create a [`TaskSpec`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskSpecInput {
    /// Concise Task title.
    pub title: String,
    /// Longer free-form description, which may be blank during drafting.
    pub description: String,
    /// Optional reason the work matters.
    pub rationale: Option<String>,
    /// Optional Definition of Done. Approval requires it to be present.
    pub definition_of_done: Option<String>,
    /// In/out scope and constraints.
    pub scope: TaskScope,
    /// Project-defined typed properties.
    pub properties: TaskProperties,
    /// Free-form Project labels used only for organization and search.
    pub labels: BTreeSet<String>,
}

/// Mutable Task intent before approval and an immutable execution-spec input after it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub(super) title: String,
    pub(super) description: String,
    pub(super) rationale: Option<String>,
    pub(super) definition_of_done: Option<String>,
    pub(super) scope: TaskScope,
    pub(super) properties: TaskProperties,
    pub(super) labels: BTreeSet<String>,
}

impl TaskSpec {
    /// Validates and creates a Task specification.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] when the title, optional text, or
    /// labels violate the bounded canonical shape.
    pub fn new(input: TaskSpecInput) -> Result<Self, DomainError> {
        validate_required_text("task.title", &input.title, 240)?;
        validate_optional_text("task.rationale", input.rationale.as_deref(), 10_000)?;
        validate_optional_text(
            "task.definition_of_done",
            input.definition_of_done.as_deref(),
            20_000,
        )?;
        if input.description.chars().count() > 50_000 {
            return Err(DomainError::InvalidValue {
                field: "task.description",
                reason: "must not exceed 50000 characters".to_owned(),
            });
        }
        for label in &input.labels {
            validate_required_text("task.labels", label, 64)?;
        }

        Ok(Self {
            title: input.title,
            description: input.description,
            rationale: input.rationale,
            definition_of_done: input.definition_of_done,
            scope: input.scope,
            properties: input.properties,
            labels: input.labels,
        })
    }

    /// Returns the concise Task title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the long description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the optional rationale.
    #[must_use]
    pub fn rationale(&self) -> Option<&str> {
        self.rationale.as_deref()
    }

    /// Returns the optional Definition of Done.
    #[must_use]
    pub fn definition_of_done(&self) -> Option<&str> {
        self.definition_of_done.as_deref()
    }

    /// Returns task boundaries and constraints.
    #[must_use]
    pub fn scope(&self) -> &TaskScope {
        &self.scope
    }

    /// Returns Project-defined typed properties.
    #[must_use]
    pub fn properties(&self) -> &TaskProperties {
        &self.properties
    }

    /// Returns free-form labels in deterministic order.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().map(String::as_str)
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.properties.validate_snapshot()?;
        Self::new(TaskSpecInput {
            title: self.title.clone(),
            description: self.description.clone(),
            rationale: self.rationale.clone(),
            definition_of_done: self.definition_of_done.clone(),
            scope: self.scope.clone(),
            properties: self.properties.clone(),
            labels: self.labels.clone(),
        })
        .map(|_| ())
    }

    pub(super) fn resolve_for_approval(
        &self,
        schema: &TaskPropertySchema,
    ) -> Result<TaskProperties, DomainError> {
        if self.definition_of_done.is_none() {
            return Err(DomainError::InvalidValue {
                field: "task.definition_of_done",
                reason: "is required before approval".to_owned(),
            });
        }
        schema.resolve(&self.properties)
    }
}

fn validate_required_text(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<(), DomainError> {
    if value.trim().is_empty() || value.chars().count() > max_len {
        return Err(DomainError::InvalidValue {
            field,
            reason: format!("must be non-blank and at most {max_len} characters"),
        });
    }
    Ok(())
}

fn validate_optional_text(
    field: &'static str,
    value: Option<&str>,
    max_len: usize,
) -> Result<(), DomainError> {
    if let Some(value) = value {
        validate_required_text(field, value, max_len)?;
    }
    Ok(())
}
