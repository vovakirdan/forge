//! Audit projection equivalent to the PostgreSQL outbox envelope.

use forge_application::RepositoryError;
use forge_domain::{AggregateRef, DomainEvent, EventId};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{MemoryEvent, MemoryOutbox, MemoryTransaction};

impl MemoryTransaction {
    pub(super) fn append_audit(&mut self, event: &DomainEvent) -> Result<EventId, RepositoryError> {
        if self
            .staged
            .events
            .iter()
            .any(|e| e.event.id() == event.id())
        {
            return Err(RepositoryError::InvalidInput {
                reason: "duplicate immutable event".to_owned(),
            });
        }
        let sequence = self
            .staged
            .events
            .iter()
            .rev()
            .find(|e| e.event.project_id() == event.project_id())
            .map_or(0, |e| e.project_sequence)
            .checked_add(1)
            .ok_or(RepositoryError::Unavailable)?;
        let Value::String(kind) =
            serde_json::to_value(event.kind()).map_err(|_| RepositoryError::Unavailable)?
        else {
            return Err(RepositoryError::Unavailable);
        };
        let aggregate_id = match event.aggregate() {
            AggregateRef::Project(id) => id.as_uuid(),
            AggregateRef::Task(id) => id.as_uuid(),
            AggregateRef::Employee(id) => id.as_uuid(),
            AggregateRef::Artifact(id) => id.as_uuid(),
            AggregateRef::Pipeline(id) => id.as_uuid(),
            AggregateRef::PipelineVersion(id) => id.as_uuid(),
        };
        let envelope = json!({
            "schema_version": event.schema_version(),
            "event_id": event.id(),
            "project_id": event.project_id(),
            "project_sequence": sequence,
            "aggregate_type": event.aggregate().aggregate_type(),
            "aggregate_id": aggregate_id,
            "aggregate_revision": event.aggregate_revision(),
            "event_type": kind,
            "actor": event.actor(),
            "command_id": event.command_id(),
            "reason": event.reason(),
            "payload": event.payload(),
            "occurred_at": event.occurred_at(),
        });
        self.staged.events.push(MemoryEvent {
            event: event.clone(),
            project_sequence: sequence,
        });
        self.staged.outbox.push(MemoryOutbox {
            id: Uuid::now_v7(),
            project_id: event.project_id(),
            event_id: event.id(),
            subject: format!("forge.v1.project.{}.event.{kind}", event.project_id()),
            envelope,
        });
        Ok(event.id())
    }
}
