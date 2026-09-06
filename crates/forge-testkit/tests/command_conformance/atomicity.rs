use anyhow::Result;
use forge_application::{CommandEnvelope, CommandError, RepositoryError, execute_in_transaction};
use forge_protocol::wire::CommandName;
use forge_storage::PostgresStore;
use forge_testkit::{
    m0::single_stage_pipeline,
    reference::{FaultPoint, FaultTransaction},
};
use serde_json::json;

use super::fixture::{Backend, BackendKind, Fixture, unavailable};

pub async fn every_write_failure_rolls_back(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    for point in [
        FaultPoint::AfterAggregateWrite,
        FaultPoint::AfterAuditAppend,
        FaultPoint::AfterReceiptInsert,
        FaultPoint::BeforeCommit,
    ] {
        let before = fixture.snapshot().await?;
        let mut pipeline = single_stage_pipeline();
        pipeline["name"] = json!(format!("Fault boundary {point:?}"));
        let envelope = fixture.envelope(
            CommandName::CreatePipeline,
            before.projects[&fixture.project_id].revision(),
            pipeline,
            &format!("fault-{point:?}"),
        );
        let failed = execute_fault(&fixture, &envelope, point).await;
        assert!(
            matches!(failed, Err(CommandError::Repository(_))),
            "fault {point:?}: {failed:?}"
        );
        assert_eq!(
            before.raw,
            fixture.snapshot().await?.raw,
            "fault {point:?} published staged writes"
        );
        fixture.execute_as(&envelope, &fixture.context).await?;
        let after = fixture.snapshot().await?;
        assert_eq!(
            after.rows("event_log").len(),
            before.rows("event_log").len() + 2
        );
        assert_eq!(
            after.rows("idempotency_keys").len(),
            before.rows("idempotency_keys").len() + 1
        );
        after.assert_audit_atomic();
    }
    Ok(())
}

pub async fn execute_fault(
    fixture: &Fixture,
    envelope: &CommandEnvelope,
    point: FaultPoint,
) -> Result<(), CommandError> {
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut transaction = FaultTransaction::new(store.begin().await, point);
            async {
                execute_in_transaction(
                    &mut transaction,
                    &fixture.context,
                    envelope,
                    &fixture.clock,
                )
                .await?;
                transaction.before_commit()?;
                transaction.into_inner().commit().await?;
                Ok(())
            }
            .await
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut transaction =
                FaultTransaction::new(store.begin().await.map_err(unavailable)?, point);
            async {
                execute_in_transaction(
                    &mut transaction,
                    &fixture.context,
                    envelope,
                    &fixture.clock,
                )
                .await?;
                transaction.before_commit()?;
                transaction
                    .into_inner()
                    .commit()
                    .await
                    .map_err(unavailable)?;
                Ok(())
            }
            .await
        }
    }
}

pub async fn composite_queue_wait_and_artifact_writes_roll_back(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let task = fixture.create_task(pipeline, "Atomic queue").await?;
    let blocker = fixture.create_task(pipeline, "Atomic dependency").await?;
    let snapshot = fixture.snapshot().await?;
    let approve = fixture.envelope(
        CommandName::ApproveTask,
        snapshot.projects[&fixture.project_id].revision(),
        json!({"task_id":task,"expected_task_revision":fixture.task(task).await?.revision().get()}),
        "atomic-approve",
    );
    assert_faults(&fixture, &approve).await?;
    fixture.execute_as(&approve, &fixture.context).await?;
    assert_eq!(fixture.snapshot().await?.rows("queue_entries").len(), 1);
    let dependency = fixture.envelope(
        CommandName::CreateDependency,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        super::commands::edge(blocker, task),
        "atomic-dependency",
    );
    assert_faults(&fixture, &dependency).await?;
    fixture.execute_as(&dependency, &fixture.context).await?;
    assert_eq!(
        fixture.task(task).await?.lifecycle(),
        forge_domain::LifecycleStatus::Waiting
    );

    let pipeline = fixture
        .pipeline(super::commands::human_evidence_pipeline())
        .await?;
    let task = fixture
        .create_task(pipeline, "Atomic artifact and completion")
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    let task_state = fixture.task(task).await?;
    let wait = task_state
        .wait_conditions()
        .next()
        .expect("human stage wait")
        .id();
    let outcome = fixture.envelope(
        CommandName::SubmitExternalStageOutcome,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"task_id":task,"expected_task_revision":task_state.revision().get(),
            "stage_id":"review","outcome":"accepted","wait_condition_id":wait,
            "artifacts":[super::commands::evidence_artifact()]}),
        "atomic-outcome",
    );
    assert_faults(&fixture, &outcome).await?;
    fixture.execute_as(&outcome, &fixture.context).await?;
    assert_eq!(
        fixture.task(task).await?.lifecycle(),
        forge_domain::LifecycleStatus::Done
    );
    assert_eq!(fixture.snapshot().await?.rows("artifacts").len(), 1);
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn assert_faults(fixture: &Fixture, envelope: &CommandEnvelope) -> Result<()> {
    let before = fixture.snapshot().await?.raw;
    for point in [
        FaultPoint::AfterAggregateWrite,
        FaultPoint::AfterAuditAppend,
        FaultPoint::AfterReceiptInsert,
        FaultPoint::BeforeCommit,
    ] {
        let result = execute_fault(fixture, envelope, point).await;
        assert!(
            matches!(
                result,
                Err(CommandError::Repository(RepositoryError::Unavailable))
            ),
            "fault {point:?} did not fire: {result:?}"
        );
        assert_eq!(
            before,
            fixture.snapshot().await?.raw,
            "fault {point:?} leaked composite writes"
        );
    }
    Ok(())
}

pub async fn dropping_successful_uncommitted_command_rolls_back(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let envelope = fixture.envelope(
        CommandName::CreatePipeline,
        1,
        single_stage_pipeline(),
        "drop-before-commit",
    );
    let before = fixture.snapshot().await?.raw;
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut transaction = store.begin().await;
            execute_in_transaction(
                &mut transaction,
                &fixture.context,
                &envelope,
                &fixture.clock,
            )
            .await?;
            drop(transaction);
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut transaction = store.begin().await?;
            execute_in_transaction(
                &mut transaction,
                &fixture.context,
                &envelope,
                &fixture.clock,
            )
            .await?;
            drop(transaction);
        }
    }
    assert_eq!(before, fixture.snapshot().await?.raw);
    fixture.execute_as(&envelope, &fixture.context).await?;
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}
