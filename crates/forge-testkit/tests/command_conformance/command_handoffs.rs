//! Manual outcomes hand over the actual stage using their retained command receipt.
use super::{
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};
use anyhow::{Context, Result};
use forge_application::CommandTransaction;
use forge_domain::{HandoffProducer, TaskHandoff};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::json;

pub async fn handoff_is_atomic_and_replayable(kind: BackendKind) -> Result<()> {
    let f = Fixture::create(kind).await?;
    let version=f.pipeline(json!({"name":"Manual handover","task_kinds":["delivery"],"entry_stage_id":"plan","max_stage_visits":8,
        "stages":[{"id":"plan","name":"Plan","executor_kind":"human","outcomes":["ready"]},{"id":"work","name":"Work","executor_kind":"employee","outcomes":["done"]}],
        "transitions":[{"from_stage_id":"plan","outcome":"ready","target":{"kind":"stage","stage_id":"work"}},{"from_stage_id":"work","outcome":"done","target":{"kind":"done"}}]})).await?;
    let task = f
        .create_task(version, "No fabricated worker for a human decision")
        .await?;
    f.task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    let state = f.task(task).await?;
    let wait = state
        .wait_conditions()
        .next()
        .context("manual stage wait")?
        .id();
    let envelope=f.envelope(CommandName::SubmitExternalStageOutcome,f.snapshot().await?.projects[&f.project_id].revision(),json!({"task_id":task,"expected_task_revision":state.revision().get(),"stage_id":"plan","outcome":"ready","wait_condition_id":wait,"artifacts":[super::commands::evidence_artifact()]}),"manual-handoff");
    assert_faults(&f, &envelope).await?;
    let receipt = f.execute_as(&envelope, &f.context).await?;
    let snapshot = f.snapshot().await?;
    let rows = snapshot.rows("task_handoffs");
    assert_eq!(rows.len(), 1);
    let handoff: TaskHandoff = serde_json::from_value(rows[0]["body"].clone())?;
    let data = handoff.data();
    assert_eq!(
        data.producer,
        HandoffProducer::Command {
            command_id: receipt.command_id.parse()?
        }
    );
    assert_eq!(data.actor, f.context.actor);
    assert_eq!(data.source_stage_id.as_str(), "plan");
    assert_eq!(data.target_stage_id.as_str(), "work");
    assert_eq!(data.artifacts.len(), 1);
    assert!(data.evidence.is_empty());
    assert!(data.work_surface_id.is_none());
    assert_eq!(
        f.execute_as(&envelope, &f.context).await?.status,
        CommandStatus::Replayed
    );
    assert_eq!(snapshot.raw, f.snapshot().await?.raw);
    match &f.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            reject_duplicate_command(&mut tx, f.project_id).await?;
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            reject_duplicate_command(&mut tx, f.project_id).await?;
        }
    }
    assert_eq!(snapshot.raw, f.snapshot().await?.raw);
    let mut forged = data.clone();
    forged.id = uuid::Uuid::now_v7();
    forged.task_id = f.create_task(version, "Unrelated task").await?;
    let forged = TaskHandoff::new(forged)?;
    let before = f.snapshot().await?.raw;
    match &f.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            assert!(tx.insert_command_handoff(&forged).await.is_err());
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            assert!(tx.insert_command_handoff(&forged).await.is_err());
            drop(tx);
            assert!(sqlx::query("UPDATE task_handoffs SET body=jsonb_set(body,'{target_stage_id}','\"plan\"') WHERE id=$1").bind(data.id).execute(pool).await.is_err());
            assert!(
                sqlx::query("DELETE FROM task_handoffs WHERE id=$1")
                    .bind(data.id)
                    .execute(pool)
                    .await
                    .is_err()
            );
            for sql in [
                "UPDATE idempotency_keys SET actor='{}'::jsonb WHERE command_id=$1",
                "UPDATE idempotency_keys SET receipt='{}'::jsonb WHERE command_id=$1",
                "UPDATE idempotency_keys SET command_name='renamed' WHERE command_id=$1",
                "UPDATE idempotency_keys SET request_hash='changed' WHERE command_id=$1",
                "DELETE FROM idempotency_keys WHERE command_id=$1",
            ] {
                assert!(
                    sqlx::query(sql)
                        .bind(receipt.command_id.parse::<uuid::Uuid>()?)
                        .execute(pool)
                        .await
                        .is_err(),
                    "linked receipt provenance must remain immutable: {sql}"
                );
            }
        }
    }
    assert_eq!(before, f.snapshot().await?.raw);
    Ok(())
}

async fn reject_duplicate_command(
    tx: &mut impl CommandTransaction,
    project: forge_domain::ProjectId,
) -> Result<()> {
    let mut receipt = tx
        .lookup_idempotency(project, "manual-handoff")
        .await?
        .context("receipt")?;
    receipt.key = "different-key-same-command".into();
    assert!(
        tx.insert_idempotency(&receipt).await.is_err(),
        "a new key must not redefine an existing command identity"
    );
    Ok(())
}
