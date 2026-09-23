use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Number;
use time::Date;

use crate::{ArtifactId, DomainError, EmployeeId, ProjectId, TaskId, ids::validate_stable_key};

/// Stable Project-defined key for a typed Task property.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PropertyKey(String);

impl PropertyKey {
    /// Validates a stable property key.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIdentifier`] for a malformed key.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_stable_key("task_property_key", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// Non-blank option used by enum and multi-enum Task properties.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PropertyChoice(String);

impl PropertyChoice {
    /// Creates a display-safe enum choice.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for blank or overly long values.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().count() > 128 {
            return Err(DomainError::InvalidValue {
                field: "task_property_choice",
                reason: "must be non-blank and at most 128 characters".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the choice label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// Typed reference from a property to another Forge aggregate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "reference_type",
    content = "reference_id",
    rename_all = "snake_case"
)]
pub enum ForgeReference {
    /// References the containing Project or another Project-visible object.
    Project(ProjectId),
    /// References another Task.
    Task(TaskId),
    /// References an immutable Artifact.
    Artifact(ArtifactId),
    /// References an Employee.
    Employee(EmployeeId),
}

impl ForgeReference {
    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Project(id) => id.validate_v7("task_property.reference.project_id"),
            Self::Task(id) => id.validate_v7("task_property.reference.task_id"),
            Self::Artifact(id) => id.validate_v7("task_property.reference.artifact_id"),
            Self::Employee(id) => id.validate_v7("task_property.reference.employee_id"),
        }
    }
}

/// Static type declared by a [`TaskPropertyDefinition`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyType {
    /// Boolean flag.
    Boolean,
    /// Arbitrary textual value.
    Text,
    /// JSON number without `NaN` or infinity.
    Number,
    /// Calendar date without a time zone.
    Date,
    /// One choice from a Project-defined option set.
    Enum,
    /// One or more choices from a Project-defined option set.
    MultiEnum,
    /// Typed Forge aggregate reference.
    Reference,
}

/// A value whose runtime shape is checked against a Project property schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PropertyValue {
    /// Boolean value.
    Boolean(bool),
    /// Text value. Blank text is allowed only for optional informational fields.
    Text(String),
    /// Lossless JSON number.
    Number(Number),
    /// Calendar date.
    Date(Date),
    /// One validated option.
    Enum(PropertyChoice),
    /// A set of validated options.
    MultiEnum(BTreeSet<PropertyChoice>),
    /// Typed Forge aggregate reference.
    Reference(ForgeReference),
}

impl PropertyValue {
    /// Returns the schema type represented by this value.
    #[must_use]
    pub const fn property_type(&self) -> PropertyType {
        match self {
            Self::Boolean(_) => PropertyType::Boolean,
            Self::Text(_) => PropertyType::Text,
            Self::Number(_) => PropertyType::Number,
            Self::Date(_) => PropertyType::Date,
            Self::Enum(_) => PropertyType::Enum,
            Self::MultiEnum(_) => PropertyType::MultiEnum,
            Self::Reference(_) => PropertyType::Reference,
        }
    }

    fn is_present(&self) -> bool {
        match self {
            Self::Text(value) => !value.trim().is_empty(),
            Self::MultiEnum(values) => !values.is_empty(),
            _ => true,
        }
    }

    /// Revalidates structural invariants after deserializing persisted or
    /// untrusted tagged property data.
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Enum(choice) => choice.validate(),
            Self::MultiEnum(choices) => {
                for choice in choices {
                    choice.validate()?;
                }
                Ok(())
            }
            Self::Reference(reference) => reference.validate(),
            Self::Boolean(_) | Self::Text(_) | Self::Number(_) | Self::Date(_) => Ok(()),
        }
    }
}

/// Project-owned typed values currently assigned to a Task.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskProperties(BTreeMap<PropertyKey, PropertyValue>);

impl TaskProperties {
    /// Creates a property map, rejecting duplicate keys instead of silently
    /// retaining the last value.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProperty`] when a key occurs twice.
    pub fn new(
        values: impl IntoIterator<Item = (PropertyKey, PropertyValue)>,
    ) -> Result<Self, DomainError> {
        let mut map = BTreeMap::new();
        for (key, value) in values {
            key.validate()?;
            value.validate()?;
            if map.insert(key.clone(), value).is_some() {
                return Err(DomainError::InvalidProperty {
                    key: key.as_str().to_owned(),
                    reason: "appears more than once".to_owned(),
                });
            }
        }
        Ok(Self(map))
    }

    /// Returns an empty property map.
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeMap::new())
    }

    /// Returns a value by stable key.
    #[must_use]
    pub fn get(&self, key: &PropertyKey) -> Option<&PropertyValue> {
        self.0.get(key)
    }

    /// Iterates deterministically by stable property key.
    pub fn iter(&self) -> impl Iterator<Item = (&PropertyKey, &PropertyValue)> {
        self.0.iter()
    }

    /// Returns the count of explicitly assigned values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether no properties were assigned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// One typed property definition belonging to a Project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPropertyDefinition {
    key: PropertyKey,
    display_name: String,
    property_type: PropertyType,
    required: bool,
    default_value: Option<PropertyValue>,
    allowed_choices: Option<BTreeSet<PropertyChoice>>,
}

impl TaskPropertyDefinition {
    /// Creates one property definition.
    ///
    /// `allowed_choices` is meaningful only for `enum` and `multi_enum` types;
    /// when present, every enum value is checked against it.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProperty`] for incompatible defaults or
    /// option-set configuration.
    pub fn new(
        key: PropertyKey,
        display_name: impl Into<String>,
        property_type: PropertyType,
        required: bool,
        default_value: Option<PropertyValue>,
        allowed_choices: Option<BTreeSet<PropertyChoice>>,
    ) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        key.validate()?;
        if display_name.trim().is_empty() || display_name.chars().count() > 200 {
            return Err(DomainError::InvalidProperty {
                key: key.as_str().to_owned(),
                reason: "display name must be non-blank and at most 200 characters".to_owned(),
            });
        }
        if let Some(default_value) = default_value.as_ref() {
            default_value.validate()?;
            validate_value_type(&key, property_type, default_value)?;
            if required && !default_value.is_present() {
                return Err(DomainError::InvalidProperty {
                    key: key.as_str().to_owned(),
                    reason: "required default value must not be empty".to_owned(),
                });
            }
        }
        validate_choice_configuration(&key, property_type, allowed_choices.as_ref())?;
        if let Some(allowed_choices) = allowed_choices.as_ref() {
            for choice in allowed_choices {
                choice.validate()?;
            }
        }
        if let (Some(default_value), Some(allowed_choices)) =
            (default_value.as_ref(), allowed_choices.as_ref())
        {
            validate_allowed_choices(&key, default_value, allowed_choices)?;
        }

        Ok(Self {
            key,
            display_name,
            property_type,
            required,
            default_value,
            allowed_choices,
        })
    }

    /// Returns the stable schema key.
    #[must_use]
    pub fn key(&self) -> &PropertyKey {
        &self.key
    }

    /// Returns the human-readable field name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the declared static type.
    #[must_use]
    pub const fn property_type(&self) -> PropertyType {
        self.property_type
    }

    /// Returns whether approval requires a non-empty value.
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    /// Returns the optional default value.
    #[must_use]
    pub fn default_value(&self) -> Option<&PropertyValue> {
        self.default_value.as_ref()
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        Self::new(
            self.key.clone(),
            self.display_name.clone(),
            self.property_type,
            self.required,
            self.default_value.clone(),
            self.allowed_choices.clone(),
        )
        .map(|_| ())
    }
}

/// Project-owned schema used to validate Task properties at approval time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPropertySchema {
    definitions: BTreeMap<PropertyKey, TaskPropertyDefinition>,
}

/// Bound on new Project schema configuration, independent of old snapshots.
pub const MAX_CONFIGURED_TASK_PROPERTIES: usize = 64;
/// Bound on the serialized schema in one management command and audit Event.
pub const MAX_CONFIGURED_TASK_PROPERTY_SCHEMA_BYTES: usize = 64 * 1024;

impl TaskPropertySchema {
    /// Validates a newly configured full schema without changing property semantics.
    pub fn validate_for_configuration(&self) -> Result<(), DomainError> {
        self.validate_snapshot()?;
        if self.definitions.len() > MAX_CONFIGURED_TASK_PROPERTIES {
            return Err(DomainError::InvalidProperty {
                key: "schema".to_owned(),
                reason: format!("may define at most {MAX_CONFIGURED_TASK_PROPERTIES} properties"),
            });
        }
        let bytes = serde_json::to_vec(self).map_err(|_| DomainError::InvalidProperty {
            key: "schema".to_owned(),
            reason: "cannot serialize schema".to_owned(),
        })?;
        if bytes.len() > MAX_CONFIGURED_TASK_PROPERTY_SCHEMA_BYTES {
            return Err(DomainError::InvalidProperty {
                key: "schema".to_owned(),
                reason: format!(
                    "must be at most {MAX_CONFIGURED_TASK_PROPERTY_SCHEMA_BYTES} bytes"
                ),
            });
        }
        Ok(())
    }

    /// Creates a schema and rejects duplicate stable keys.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProperty`] if a key appears more than once.
    pub fn new(
        definitions: impl IntoIterator<Item = TaskPropertyDefinition>,
    ) -> Result<Self, DomainError> {
        let mut values = BTreeMap::new();
        for definition in definitions {
            let key = definition.key.clone();
            if values.insert(key.clone(), definition).is_some() {
                return Err(DomainError::InvalidProperty {
                    key: key.as_str().to_owned(),
                    reason: "schema defines the key more than once".to_owned(),
                });
            }
        }
        Ok(Self {
            definitions: values,
        })
    }

    /// Resolves defaults and verifies a Task's values against this schema.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProperty`] for unknown, missing, malformed,
    /// or type-incompatible values.
    pub fn resolve(&self, properties: &TaskProperties) -> Result<TaskProperties, DomainError> {
        let mut resolved = BTreeMap::new();

        for (key, definition) in &self.definitions {
            if let Some(value) = properties.0.get(key) {
                self.validate_value(key, definition, value)?;
                resolved.insert(key.clone(), value.clone());
            } else if let Some(default_value) = definition.default_value.as_ref() {
                resolved.insert(key.clone(), default_value.clone());
            } else if definition.required {
                return Err(DomainError::InvalidProperty {
                    key: key.as_str().to_owned(),
                    reason: "required value is missing".to_owned(),
                });
            }
        }

        for key in properties.0.keys() {
            if !self.definitions.contains_key(key) {
                return Err(DomainError::InvalidProperty {
                    key: key.as_str().to_owned(),
                    reason: "is not defined by the project schema".to_owned(),
                });
            }
        }

        Ok(TaskProperties(resolved))
    }

    /// Returns a definition by stable key.
    #[must_use]
    pub fn get(&self, key: &PropertyKey) -> Option<&TaskPropertyDefinition> {
        self.definitions.get(key)
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        for (key, definition) in &self.definitions {
            if key != definition.key() {
                return Err(DomainError::InvalidProperty {
                    key: key.as_str().to_owned(),
                    reason: "definition key does not match schema map key".to_owned(),
                });
            }
            definition.validate_snapshot()?;
        }
        Self::new(self.definitions.values().cloned()).map(|_| ())
    }

    fn validate_value(
        &self,
        key: &PropertyKey,
        definition: &TaskPropertyDefinition,
        value: &PropertyValue,
    ) -> Result<(), DomainError> {
        validate_value_type(key, definition.property_type, value)?;
        if definition.required && !value.is_present() {
            return Err(DomainError::InvalidProperty {
                key: key.as_str().to_owned(),
                reason: "required value must not be empty".to_owned(),
            });
        }
        if let Some(allowed_choices) = definition.allowed_choices.as_ref() {
            validate_allowed_choices(key, value, allowed_choices)?;
        }
        Ok(())
    }
}

fn validate_value_type(
    key: &PropertyKey,
    expected: PropertyType,
    value: &PropertyValue,
) -> Result<(), DomainError> {
    if value.property_type() == expected {
        Ok(())
    } else {
        Err(DomainError::InvalidProperty {
            key: key.as_str().to_owned(),
            reason: format!("expected {expected:?}, got {:?}", value.property_type()),
        })
    }
}

fn validate_choice_configuration(
    key: &PropertyKey,
    property_type: PropertyType,
    allowed_choices: Option<&BTreeSet<PropertyChoice>>,
) -> Result<(), DomainError> {
    match (property_type, allowed_choices) {
        (PropertyType::Enum | PropertyType::MultiEnum, None) => Ok(()),
        (PropertyType::Enum | PropertyType::MultiEnum, Some(choices)) if !choices.is_empty() => {
            Ok(())
        }
        (
            PropertyType::Boolean
            | PropertyType::Text
            | PropertyType::Number
            | PropertyType::Date
            | PropertyType::Reference,
            None,
        ) => Ok(()),
        _ => Err(DomainError::InvalidProperty {
            key: key.as_str().to_owned(),
            reason: "allowed choices are only valid as a non-empty set for enum types".to_owned(),
        }),
    }
}

fn validate_allowed_choices(
    key: &PropertyKey,
    value: &PropertyValue,
    allowed_choices: &BTreeSet<PropertyChoice>,
) -> Result<(), DomainError> {
    let allowed = match value {
        PropertyValue::Enum(choice) => allowed_choices.contains(choice),
        PropertyValue::MultiEnum(choices) => choices
            .iter()
            .all(|choice| allowed_choices.contains(choice)),
        _ => true,
    };
    if allowed {
        return Ok(());
    }
    Err(DomainError::InvalidProperty {
        key: key.as_str().to_owned(),
        reason: "contains a choice that is not allowed by the project schema".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::{
        PropertyKey, PropertyType, PropertyValue, TaskProperties, TaskPropertyDefinition,
        TaskPropertySchema,
    };

    #[test]
    fn schema_rejects_missing_required_property() {
        let definition = TaskPropertyDefinition::new(
            PropertyKey::new("contains_migrations").expect("valid key"),
            "Contains migrations",
            PropertyType::Boolean,
            true,
            None,
            None,
        )
        .expect("valid definition");
        let schema = TaskPropertySchema::new([definition]).expect("valid schema");

        let result = schema.resolve(&TaskProperties::empty());

        assert!(result.is_err());
    }

    #[test]
    fn required_property_rejects_an_empty_default() {
        let definition = TaskPropertyDefinition::new(
            PropertyKey::new("migration_plan").expect("valid key"),
            "Migration plan",
            PropertyType::Text,
            true,
            Some(PropertyValue::Text("   ".to_owned())),
            None,
        );

        assert!(definition.is_err());
    }

    #[test]
    fn snapshot_rejects_a_blank_deserialized_enum_choice() {
        let properties: TaskProperties = serde_json::from_value(serde_json::json!({
            "strategy": {"type": "enum", "value": "   "}
        }))
        .expect("raw serde accepts the fixture before validation");

        assert!(properties.validate_snapshot().is_err());
    }

    #[test]
    fn snapshot_rejects_a_non_v7_deserialized_reference() {
        let properties: TaskProperties = serde_json::from_value(serde_json::json!({
            "related_task": {
                "type": "reference",
                "value": {
                    "reference_type": "task",
                    "reference_id": "00000000-0000-0000-0000-000000000000"
                }
            }
        }))
        .expect("raw serde accepts the fixture before validation");

        assert!(properties.validate_snapshot().is_err());
    }

    #[test]
    fn date_property_values_keep_the_existing_year_and_ordinal_wire_shape() {
        let encoded = serde_json::to_value(PropertyValue::Date(date!(2026 - 09 - 04)))
            .expect("date property should serialize");

        assert_eq!(
            encoded,
            serde_json::json!({"type": "date", "value": [2026, 247]})
        );
    }

    #[test]
    fn schema_rejects_a_value_with_the_wrong_type() {
        let key = PropertyKey::new("contains_migrations").expect("valid key");
        let definition = TaskPropertyDefinition::new(
            key.clone(),
            "Contains migrations",
            PropertyType::Boolean,
            false,
            None,
            None,
        )
        .expect("valid definition");
        let schema = TaskPropertySchema::new([definition]).expect("valid schema");
        let properties = TaskProperties::new([(key, PropertyValue::Text("yes".to_owned()))])
            .expect("unique property key");

        let result = schema.resolve(&properties);

        assert!(result.is_err());
    }
}
