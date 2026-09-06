use forge_application::{
    CommandContext, CommandEnvelope, CommandTransaction, execute_in_transaction,
};
use forge_domain::{Actor, ActorId, Project, ProjectId, Timestamp};
use forge_protocol::wire::{CommandName, CommandRequest};
use serde_json::json;

use super::{ManualClock, MemoryStore};

#[tokio::test]
async fn dropped_transaction_does_not_publish_staged_project() {
    let store = MemoryStore::default();
    let project = Project::new(ProjectId::new(), "discarded", Timestamp::now_utc()).unwrap();
    let mut transaction = store.begin().await;
    transaction.insert_project(&project).await.unwrap();
    assert!(transaction.snapshot().projects.contains_key(&project.id()));
    drop(transaction);
    assert!(store.snapshot().await.projects.is_empty());
}

#[tokio::test]
async fn reference_outbox_uses_the_same_envelope_as_postgres_projection() {
    let store = MemoryStore::default();
    let project = ProjectId::new();
    let actor = Actor::human(ActorId::new());
    let context = CommandContext::local_human(project, actor, Actor::core(ActorId::new()));
    let clock = ManualClock::new(Timestamp::now_utc());
    let envelope = CommandEnvelope::parse(
        CommandName::CreateProject,
        CommandRequest {
            project_id: project.to_string(),
            expected_revision: 0,
            payload: json!({"name":"outbox projection"})
                .as_object()
                .unwrap()
                .clone(),
        },
        "outbox-parity",
    )
    .unwrap();
    let mut transaction = store.begin().await;
    execute_in_transaction(&mut transaction, &context, &envelope, &clock)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let snapshot = store.snapshot().await;
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.outbox.len(), 1);
    let event = &snapshot.events[0];
    let postgres =
        forge_storage::StoredEvent::from_domain(&event.event, event.project_sequence).unwrap();
    assert_eq!(snapshot.outbox[0].envelope, postgres.envelope());
    assert_eq!(snapshot.outbox[0].subject, postgres.subject());
}
