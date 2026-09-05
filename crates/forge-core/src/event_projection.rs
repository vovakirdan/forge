//! Conversion of durable Event rows into the public SSE contract.

use forge_domain::AggregateType;
use forge_protocol::wire::{ActorReference, AggregateReference, EventEnvelope};
use forge_storage::StoredEvent;
use serde_json::{Map, Value};
use time::format_description::well_known::Rfc3339;

use crate::CoreError;

/// Projects one durable Event into the strict public SSE shape without making
/// storage depend on the transport crate.
pub fn event_envelope_from_stored_event(event: &StoredEvent) -> Result<EventEnvelope, CoreError> {
    Ok(EventEnvelope {
        schema_version: event.schema_version,
        event_id: event.id.to_string(),
        project_id: event.project_id.to_string(),
        project_sequence: event.project_sequence,
        event_type: event.event_type.clone(),
        aggregate: AggregateReference {
            kind: aggregate_kind(event.aggregate_type).to_owned(),
            id: event.aggregate_id.to_string(),
            revision: Some(event.aggregate_revision),
        },
        occurred_at: event
            .occurred_at
            .as_offset_date_time()
            .format(&Rfc3339)
            .map_err(|_| CoreError::InvalidTransport {
                field: "event.occurred_at",
                reason: "cannot be encoded as RFC 3339".to_owned(),
            })?,
        actor: public_actor(&event.actor)?,
        command_id: event.command_id.map(|command_id| command_id.to_string()),
        audit_reason: event.reason.clone(),
        payload: public_payload(&event.payload)?,
    })
}

fn aggregate_kind(aggregate_type: AggregateType) -> &'static str {
    match aggregate_type {
        AggregateType::Project => "project",
        AggregateType::Task => "task",
        AggregateType::Employee => "employee",
        AggregateType::Artifact => "artifact",
        AggregateType::Pipeline => "pipeline",
        AggregateType::PipelineVersion => "pipeline_version",
    }
}

fn public_actor(actor: &Value) -> Result<ActorReference, CoreError> {
    serde_json::from_value(actor.clone()).map_err(|_| CoreError::InvalidTransport {
        field: "event.actor",
        reason: "must match the public actor reference".to_owned(),
    })
}

fn public_payload(payload: &Value) -> Result<Map<String, Value>, CoreError> {
    payload
        .as_object()
        .cloned()
        .ok_or_else(|| CoreError::InvalidTransport {
            field: "event.payload",
            reason: "must be a JSON object".to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        Actor, ActorId, AggregateType, CommandId, EventId, ProjectId, TaskId, Timestamp,
    };
    use forge_protocol::wire::ActorKind;
    use forge_storage::StoredEvent;
    use serde_json::json;

    use super::event_envelope_from_stored_event;

    #[test]
    fn cancellation_event_projection_preserves_audit_reason_and_nested_references() {
        let actor_id = ActorId::new();
        let command_id = CommandId::new();
        let task_id = TaskId::new();
        let event = StoredEvent {
            id: EventId::new(),
            project_id: ProjectId::new(),
            project_sequence: 42,
            aggregate_type: AggregateType::Task,
            aggregate_id: task_id.as_uuid(),
            aggregate_revision: 7,
            event_type: "task_cancelled".to_owned(),
            schema_version: 1,
            actor: serde_json::to_value(Actor::human(actor_id)).expect("test actor serializes"),
            command_id: Some(command_id),
            reason: Some("operator stopped obsolete work".to_owned()),
            payload: json!({"cancellation_reason_key": "obsolete"}),
            occurred_at: Timestamp::now_utc(),
        };

        let actual = event_envelope_from_stored_event(&event).expect("valid stored event maps");
        let encoded = serde_json::to_value(actual).expect("public event serializes");

        assert_eq!(encoded["audit_reason"], "operator stopped obsolete work");
        assert_eq!(
            encoded["aggregate"],
            json!({"kind": "task", "id": task_id.to_string(), "revision": 7})
        );
        assert_eq!(
            encoded["actor"],
            json!({"kind": ActorKind::Human, "id": actor_id.to_string()})
        );
        assert_eq!(encoded["command_id"], command_id.to_string());
    }

    #[test]
    fn projection_preserves_absent_durable_audit_fields_as_null() {
        let project_id = ProjectId::new();
        let event = StoredEvent {
            id: EventId::new(),
            project_id,
            project_sequence: 1,
            aggregate_type: AggregateType::Project,
            aggregate_id: project_id.as_uuid(),
            aggregate_revision: 1,
            event_type: "project_execution_stopped".to_owned(),
            schema_version: 1,
            actor: serde_json::to_value(Actor::human(ActorId::new()))
                .expect("test actor serializes"),
            command_id: None,
            reason: None,
            payload: json!({}),
            occurred_at: Timestamp::now_utc(),
        };

        let actual = event_envelope_from_stored_event(&event).expect("valid stored event maps");
        let encoded = serde_json::to_value(actual).expect("public event serializes");

        assert!(encoded["command_id"].is_null());
        assert!(encoded["audit_reason"].is_null());
    }
}
