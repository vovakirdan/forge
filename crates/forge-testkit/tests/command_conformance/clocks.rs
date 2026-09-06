use std::{
    future::{Future, poll_fn},
    task::Poll,
};

use anyhow::Result;
use forge_application::{CommandTransaction, execute_in_transaction};
use forge_domain::{LifecycleStatus, Timestamp};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;

use super::fixture::{Backend, BackendKind, Fixture, base_time};

pub async fn canonical_and_operational_time_are_distinct(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let future = Timestamp::from_offset_date_time(
        base_time().as_offset_date_time() + time::Duration::hours(1),
    );
    fixture.clock.set(future);
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(pipeline, "Causal time").await?;
    fixture.clock.set(base_time());
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    let snapshot = fixture.snapshot().await?;
    assert_eq!(snapshot.projects[&fixture.project_id].updated_at(), future);
    assert_eq!(snapshot.tasks[&task].updated_at(), future);
    assert_eq!(snapshot.tasks[&task].lifecycle(), LifecycleStatus::Ready);
    let queue = snapshot.rows("queue_entries");
    assert_eq!(queue.len(), 1);
    // PostgreSQL stores the deadline as timestamptz, memory uses the domain wire shape.
    match &fixture.backend {
        Backend::Memory(_) => assert_eq!(queue[0]["eligible_at"], json!(base_time())),
        Backend::Postgres(pool) => {
            let eligible: time::OffsetDateTime =
                sqlx::query_scalar("SELECT eligible_at FROM queue_entries WHERE task_id=$1")
                    .bind(task.as_uuid())
                    .fetch_one(pool)
                    .await?;
            assert_eq!(eligible, base_time().as_offset_date_time());
        }
    }
    let last = snapshot
        .rows("outbox")
        .iter()
        .find(|entry| entry["envelope"]["event_type"] == "task_approved")
        .expect("approval outbox");
    assert_eq!(last["envelope"]["occurred_at"], json!(future));
    fixture.clock.set(future);
    fixture
        .task_command(
            CommandName::SetTaskPriority,
            task,
            json!({"priority":"high"}),
        )
        .await?;
    assert_eq!(
        fixture.task(task).await?.updated_at(),
        future,
        "equal clock is valid"
    );
    let later =
        Timestamp::from_offset_date_time(future.as_offset_date_time() + time::Duration::seconds(1));
    fixture.clock.set(later);
    fixture
        .task_command(
            CommandName::SetTaskPriority,
            task,
            json!({"priority":"normal"}),
        )
        .await?;
    assert_eq!(fixture.task(task).await?.updated_at(), later);
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

#[tokio::test]
async fn memory_samples_command_time_after_the_serializing_lock() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Memory).await?;
    let Backend::Memory(store) = &fixture.backend else {
        unreachable!("memory fixture")
    };
    let mut held = store.begin().await;
    held.lock_project_creation(fixture.project_id).await?;
    held.lock_project(fixture.project_id).await?;
    let next = fixture.envelope(
        CommandName::StopProjectExecution,
        2,
        json!({}),
        "blocked-command",
    );
    let mut waiting = Box::pin(fixture.execute_as(&next, &fixture.context));
    poll_fn(|cx| {
        assert!(
            waiting.as_mut().poll(cx).is_pending(),
            "command bypassed transaction lock"
        );
        Poll::Ready(())
    })
    .await;
    let future = Timestamp::from_offset_date_time(
        base_time().as_offset_date_time() + time::Duration::hours(1),
    );
    fixture.clock.set(future);
    let first = fixture.envelope(
        CommandName::StartProjectExecution,
        1,
        json!({}),
        "held-writer",
    );
    execute_in_transaction(&mut held, &fixture.context, &first, &fixture.clock).await?;
    held.commit().await?;
    fixture.clock.set(base_time());
    waiting.await?;
    assert_eq!(
        fixture.snapshot().await?.projects[&fixture.project_id].updated_at(),
        future
    );
    Ok(())
}
