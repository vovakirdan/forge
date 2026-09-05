//! Small constructors for canonical Core audit events.

use forge_domain::{
    Actor, AggregateRef, CommandId, DomainError, DomainEvent, DomainEventInput, DomainEventKind,
    EventId, ProjectId, Timestamp,
};
use forge_protocol::wire::EVENT_SCHEMA_VERSION;
use serde_json::{Map, Value};

/// Creates one immutable event using Core's authoritative receipt time.
#[expect(
    clippy::too_many_arguments,
    reason = "an immutable audit event deliberately exposes every canonical field"
)]
pub(crate) fn event(
    project_id: ProjectId,
    aggregate: AggregateRef,
    aggregate_revision: u64,
    kind: DomainEventKind,
    actor: Actor,
    command_id: CommandId,
    reason: Option<String>,
    payload: Map<String, Value>,
    occurred_at: Timestamp,
) -> Result<DomainEvent, DomainError> {
    DomainEvent::new(
        EventId::new(),
        DomainEventInput {
            project_id,
            aggregate,
            aggregate_revision,
            kind,
            actor,
            command_id,
            reason,
            occurred_at,
            schema_version: EVENT_SCHEMA_VERSION,
            payload: Value::Object(payload),
        },
    )
}

/// Builds an object payload from stable field/value pairs.
#[must_use]
pub(crate) fn event_payload(
    entries: impl IntoIterator<Item = (&'static str, Value)>,
) -> Map<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}
