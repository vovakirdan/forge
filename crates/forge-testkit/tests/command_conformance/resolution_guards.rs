//! Managed questions cannot be cleared by generic resume or a pre-existing timer.
use super::{
    fixture::{Backend, BackendKind, Fixture},
    resolution::{escalation, raise, setup},
};
use anyhow::{Context, Result};
use forge_application::{Clock, CommandTransaction};
use forge_domain::{
    ResumeRejection, ScheduledResumeState, TaskId, TaskResumeSchedule, TaskWaitCondition,
    TaskWaitKind, Timestamp, WaitConditionId,
};
use forge_protocol::wire::CommandName;
use forge_storage::PostgresStore;
use serde_json::json;
use uuid::Uuid;

fn future(fixture: &Fixture) -> Timestamp {
    Timestamp::from_offset_date_time(
        fixture.clock.now().as_offset_date_time() + time::Duration::hours(1),
    )
}
fn deadline(fixture: &Fixture) -> String {
    future(fixture)
        .as_offset_date_time()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

pub async fn managed_wait_requires_resolution_but_orphan_wait_is_compatible(
    kind: BackendKind,
) -> Result<()> {
    let (fixture, task) = setup(kind).await?;
    let id = raise(&fixture, task, json!({"category":"action_approval"})).await?;
    let wait = escalation(&fixture, id)
        .await?
        .source
        .task()
        .context("Task source")?
        .wait_condition_id;
    let before = fixture.snapshot().await?.raw;
    assert!(
        fixture
            .task_command(
                CommandName::ResumeTask,
                task,
                json!({"wait_condition_id":wait})
            )
            .await
            .is_err()
    );
    assert!(fixture.task_command(CommandName::ScheduleTaskResume,task,json!({"wait_condition_id":wait,"not_before":deadline(&fixture),"reason":"Timer cannot authorize a dangerous action"})).await.is_err());
    assert_eq!(before, fixture.snapshot().await?.raw);
    let (legacy, task) = setup(kind).await?;
    let orphan = orphan_wait(&legacy, task).await?;
    legacy
        .task_command(
            CommandName::ResumeTask,
            task,
            json!({"wait_condition_id":orphan}),
        )
        .await?;
    let orphan = orphan_wait(&legacy, task).await?;
    legacy.task_command(CommandName::ScheduleTaskResume,task,json!({"wait_condition_id":orphan,"not_before":deadline(&legacy),"reason":"Legacy owner authorization"})).await?;
    Ok(())
}

async fn orphan_wait(fixture: &Fixture, task: TaskId) -> Result<WaitConditionId> {
    let wait = WaitConditionId::new();
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            insert_orphan(&mut tx, fixture, task, wait).await?;
            tx.commit().await?;
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            insert_orphan(&mut tx, fixture, task, wait).await?;
            tx.commit().await?;
        }
    }
    Ok(wait)
}
async fn insert_orphan(
    tx: &mut impl CommandTransaction,
    fixture: &Fixture,
    task: TaskId,
    wait: WaitConditionId,
) -> Result<()> {
    let mut project = tx
        .lock_project(fixture.project_id)
        .await?
        .context("Project")?;
    let mut stored = tx.lock_task(task).await?.context("Task")?;
    let previous = stored.task.revision().get();
    let now = project.updated_at();
    stored.task.add_wait_condition(
        TaskWaitCondition::new(
            wait,
            TaskWaitKind::EscalationPending,
            Some("Synthetic legacy M1 wait".into()),
            fixture.context.actor,
            now,
        )?,
        now,
    )?;
    forge_application::engine::task_support::persist_task_and_project(
        tx,
        &mut project,
        &stored.task,
        stored.persistence,
        previous,
        now,
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; no provider"]
async fn resolver_existing_alarm_cannot_authorize_managed_question() -> Result<()> {
    let (fixture, task) = setup(BackendKind::Postgres).await?;
    fixture
        .execute(CommandName::StartProjectExecution, json!({}))
        .await?;
    let id = raise(&fixture, task, json!({"category":"action_approval"})).await?;
    let question = escalation(&fixture, id).await?;
    let source = question.source.task().context("Task source")?;
    let before = fixture.task(task).await?;
    let schedule = TaskResumeSchedule {
        id: Uuid::now_v7(),
        project_id: fixture.project_id,
        task_id: task,
        expected_task_revision: before.revision().get(),
        pipeline_version_id: source.pipeline_version_id,
        stage_id: source.stage_id.clone(),
        stage_visit: source.stage_visit,
        wait_condition_id: source.wait_condition_id,
        not_before: future(&fixture),
        reason: "Synthetic pre-existing intent".into(),
        created_by: fixture.context.actor,
        created_at: fixture.clock.now(),
        state: ScheduledResumeState::Pending,
    };
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!()
    };
    let store = PostgresStore::from_pool(pool.clone());
    let mut tx = store.begin().await?;
    tx.lock_project(fixture.project_id).await?;
    tx.insert_task_resume_schedule(&schedule).await?;
    tx.commit().await?;
    assert_eq!(
        fixture
            .core(pool)
            .with_fake_runtime()
            .resume_due_tasks(schedule.not_before)
            .await?,
        1
    );
    assert_eq!(before, fixture.task(task).await?);
    let mut tx = store.begin().await?;
    let result = tx
        .lock_task_resume_schedule(schedule.id)
        .await?
        .context("schedule result")?;
    assert!(matches!(
        result.state,
        ScheduledResumeState::Rejected {
            reason: ResumeRejection::ResolutionRequired,
            ..
        }
    ));
    assert_eq!(tx.load_escalation(id).await?.context("question")?, question);
    Ok(())
}
