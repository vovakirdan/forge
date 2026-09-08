//! Explicit scheduler fixtures, not an in-memory runtime or a provider simulation.

use anyhow::{Context, Result};
use forge_application::{Clock, CommandTransaction, ports::ActiveRun};
use forge_domain::{EmployeeId, LifecycleStatus, TaskId, Timestamp};
use forge_protocol::wire::CommandName;
use forge_storage::{LeaseRunRequest, PostgresStore, QueueEntry, QueueState};
use forge_testkit::{
    m0::single_stage_pipeline,
    reference::{MemoryQueueState, MemoryRun},
};
use serde_json::json;
use uuid::Uuid;

use super::{
    atomicity::assert_faults,
    commands::edge,
    fixture::{Backend, BackendKind, Fixture},
};

pub struct RunningTask {
    pub fixture: Fixture,
    pub task: TaskId,
    pub blocker: TaskId,
    pub run: ActiveRun,
    pub queue: Uuid,
}

pub async fn running_task(kind: BackendKind) -> Result<RunningTask> {
    running_task_with_visit(kind, None).await
}

pub async fn running_task_with_visit(
    kind: BackendKind,
    stage_visit: Option<forge_domain::StageVisit>,
) -> Result<RunningTask> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let employee: EmployeeId = fixture
        .execute(
            CommandName::CreateEmployee,
            json!({
                "name":"Fenced worker", "role":"m0_worker", "stage_eligibility":{"mode":"any"}
            }),
        )
        .await?
        .resource
        .context("employee")?
        .id
        .parse()?;
    let task = fixture.create_task(pipeline, "Already running").await?;
    let blocker = fixture.create_task(pipeline, "New blocker").await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    fixture
        .execute(CommandName::StartProjectExecution, json!({}))
        .await?;
    let run_id = Uuid::now_v7();
    let (run, queue) = match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            let queue = tx
                .snapshot_mut()
                .queue
                .iter_mut()
                .find(|q| q.input.task_id == task && q.state == MemoryQueueState::Queued)
                .context("queued task")?;
            queue.state = MemoryQueueState::Leased;
            let queue_id = queue.id;
            let run = ActiveRun {
                id: run_id,
                project_id: fixture.project_id,
                assignment: forge_domain::ExecutionAssignment::TaskStage(
                    forge_domain::TaskStageAssignment {
                        task_id: task,
                        queue_entry_id: queue_id,
                        stage_id: forge_domain::StageId::new("work")?,
                    },
                ),
                stage_visit,
                employee_id: Some(employee),
                lease_fencing_token: 7,
                environment_epoch: 1,
            };
            tx.snapshot_mut().runs.push(MemoryRun {
                scope: run.clone(),
                queue_entry_id: queue_id,
                lease_active: true,
                reservation_held: true,
                stop_requested: None,
            });
            mark_task_started(&mut tx, &fixture, task).await?;
            tx.commit().await?;
            (run, queue_id)
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            // Claim is fixture setup, not a scheduler test. In particular it does
            // not compare ManualClock time against PostgreSQL clock_timestamp().
            let queue_id: Uuid = sqlx::query_scalar(
                "UPDATE queue_entries SET queue_state='leased' WHERE task_id=$1 AND queue_state='queued' RETURNING id"
            ).bind(task.as_uuid()).fetch_one(pool).await?;
            let project = store
                .load_project(fixture.project_id)
                .await?
                .context("project")?;
            let stored = store.load_task(task).await?.context("task")?;
            // No revision or priority changed since approval; the production
            // projection constructs the same immutable queued input.
            let queue = QueueEntry {
                id: queue_id,
                input: forge_application::engine::scheduler::queue_input(
                    &project,
                    &stored.task,
                    &stored.persistence,
                    fixture.clock.now(),
                )?,
                state: QueueState::Leased,
            };
            let mut tx = store.begin().await?;
            // Physical Lease metadata follows database time, independently of
            // the fixed canonical command clock (and its intentional rollback).
            let lease_expiry: time::OffsetDateTime =
                sqlx::query_scalar("SELECT clock_timestamp() + interval '1 hour'")
                    .fetch_one(pool)
                    .await?;
            let request = LeaseRunRequest {
                queue_entry: queue,
                employee_id: employee,
                lease_id: Uuid::now_v7(),
                run_id,
                lease_expires_at: Timestamp::from_offset_date_time(lease_expiry),
                task_work_surface_id: None,
                lease_scope: json!({"fixture":true}),
                resource_reservation: json!({}),
                run_spec_version: 1,
                run_spec: json!({}),
                context_manifest: match stage_visit {
                    None => json!({}),
                    Some(visit) => serde_json::to_value(forge_domain::ContextSnapshot::new(
                        forge_domain::ContextSnapshotInput {
                            context_snapshot_id: Uuid::now_v7(),
                            project_id: fixture.project_id,
                            task_id: task,
                            run_id,
                            employee_id: employee,
                            pipeline_version_id: pipeline,
                            stage_id: forge_domain::StageId::new("work")?,
                            stage_visit: Some(visit.get()),
                            task_revision_before_dispatch: stored.task.revision().get(),
                            task_spec: stored.task.spec().clone(),
                            system_policy_revision: "fixture/v1".into(),
                            employee_prompt_revision: "employee/v1".into(),
                            capability_grants: vec![],
                            tool_catalog_revision: "fixture/v1".into(),
                            run_spec_id: Uuid::now_v7(),
                            prior_handoff: None,
                            artifacts: vec![],
                            control_instruction: None,
                            created_at: fixture.clock.now(),
                        },
                    )?)?,
                },
            };
            let provisioned = tx.create_lease_and_run(&request).await?;
            tx.reserve_environment(run_id, None).await?;
            mark_task_started(&mut tx, &fixture, task).await?;
            tx.commit().await?;
            (
                ActiveRun {
                    id: run_id,
                    project_id: fixture.project_id,
                    assignment: forge_domain::ExecutionAssignment::TaskStage(
                        forge_domain::TaskStageAssignment {
                            task_id: task,
                            queue_entry_id: queue_id,
                            stage_id: forge_domain::StageId::new("work")?,
                        },
                    ),
                    stage_visit,
                    employee_id: Some(employee),
                    lease_fencing_token: provisioned.lease_fencing_token,
                    environment_epoch: provisioned.environment_epoch,
                },
                queue_id,
            )
        }
    };
    assert_eq!(
        fixture.task(task).await?.lifecycle(),
        LifecycleStatus::InProgress
    );
    Ok(RunningTask {
        fixture,
        task,
        blocker,
        run,
        queue,
    })
}

async fn mark_task_started(
    tx: &mut impl CommandTransaction,
    fixture: &Fixture,
    id: TaskId,
) -> Result<()> {
    let mut project = tx
        .lock_project(fixture.project_id)
        .await?
        .context("project")?;
    let stored = tx.lock_task(id).await?.context("task")?;
    let mut task = stored.task;
    let version = tx
        .lock_pipeline_version(task.pipeline().pipeline_version_id())
        .await?
        .context("version")?;
    let previous = task.revision().get();
    version.start_entry_employee_stage(&mut task, fixture.clock.now())?;
    task.record_run_attempt(fixture.clock.now())?;
    let mut persistence = stored.persistence;
    persistence.attempt_count += 1;
    forge_application::engine::task_support::persist_task_and_project(
        tx,
        &mut project,
        &task,
        persistence,
        previous,
        fixture.clock.now(),
    )
    .await?;
    Ok(())
}

pub async fn stop_cancel_and_dependency_preserve_physical_ownership(
    kind: BackendKind,
) -> Result<()> {
    for command in [
        CommandName::StopProjectExecution,
        CommandName::CancelTask,
        CommandName::CreateDependency,
    ] {
        let setup = running_task(kind).await?;
        let fixture = &setup.fixture;
        reject_stale_fences(&setup).await?;
        let payload = match command {
            CommandName::StopProjectExecution => json!({"reason":"fixture stop"}),
            CommandName::CancelTask => json!({"task_id":setup.task,
                "expected_task_revision":fixture.task(setup.task).await?.revision().get(),
                "cancellation_reason_key":"unspecified"}),
            CommandName::CreateDependency => edge(setup.blocker, setup.task),
            _ => unreachable!("fixed command table"),
        };
        let envelope = fixture.envelope(
            command,
            fixture.snapshot().await?.projects[&fixture.project_id].revision(),
            payload,
            "fenced-stop",
        );
        // This also proves failed commands restore staged stop flags and Lease-backed queue state.
        assert_faults(fixture, &envelope).await?;
        let before_stop = fixture.snapshot().await?.raw;
        fixture.execute_as(&envelope, &fixture.context).await?;
        let expected = if command == CommandName::CancelTask {
            LifecycleStatus::Cancelled
        } else {
            LifecycleStatus::Waiting
        };
        assert_eq!(fixture.task(setup.task).await?.lifecycle(), expected);
        assert_retained_stop(&setup).await?;
        if matches!(fixture.backend, Backend::Postgres(_)) {
            let after_stop = fixture.snapshot().await?.raw;
            for table in ["leases", "run_environment_reservations"] {
                assert_eq!(
                    before_stop[table], after_stop[table],
                    "stop must retain exact {table}"
                );
            }
        }
        // A retry cannot produce a second stop event or release ownership.
        let before = fixture.snapshot().await?.raw;
        fixture.execute_as(&envelope, &fixture.context).await?;
        assert_eq!(fixture.snapshot().await?.raw, before);
    }
    Ok(())
}

async fn reject_stale_fences(setup: &RunningTask) -> Result<()> {
    let before = setup.fixture.snapshot().await?.raw;
    match &setup.fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            reject_stale_in_transaction(&mut tx, &setup.run).await?;
            tx.commit().await?;
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            reject_stale_in_transaction(&mut tx, &setup.run).await?;
            tx.commit().await?;
        }
    }
    assert_eq!(setup.fixture.snapshot().await?.raw, before);
    Ok(())
}

async fn reject_stale_in_transaction(
    tx: &mut impl CommandTransaction,
    run: &ActiveRun,
) -> Result<()> {
    for (fence, epoch) in [
        (
            run.lease_fencing_token
                .checked_add(1)
                .context("fence increment")?,
            run.environment_epoch,
        ),
        (
            run.lease_fencing_token,
            run.environment_epoch
                .checked_add(1)
                .context("epoch increment")?,
        ),
    ] {
        assert!(!tx.request_run_stop(run.id, fence, epoch, false).await?);
        assert!(
            !tx.cancel_leased_queue_for_fenced_run(run.id, fence, epoch)
                .await?
        );
    }
    Ok(())
}

async fn assert_retained_stop(setup: &RunningTask) -> Result<()> {
    match &setup.fixture.backend {
        Backend::Memory(store) => {
            let snapshot = store.snapshot().await;
            let run = snapshot
                .runs
                .iter()
                .find(|r| r.scope.id == setup.run.id)
                .context("run")?;
            assert_eq!(run.stop_requested, Some(false));
            assert!(run.lease_active && run.reservation_held);
            assert_eq!(run.scope.lease_fencing_token, setup.run.lease_fencing_token);
            assert_eq!(run.scope.environment_epoch, setup.run.environment_epoch);
            assert_eq!(
                snapshot
                    .queue
                    .iter()
                    .find(|q| q.id == setup.queue)
                    .context("queue")?
                    .state,
                MemoryQueueState::Cancelled
            );
        }
        Backend::Postgres(pool) => {
            let state: (String,String,String,bool,i64,i64) = sqlx::query_as(
                "SELECT r.desired_state,l.lease_state,q.queue_state,p.released_at IS NULL,r.lease_fencing_token,r.environment_epoch FROM runs r JOIN leases l ON l.id=r.lease_id JOIN queue_entries q ON q.id=r.queue_entry_id JOIN run_environment_reservations p ON p.run_id=r.id WHERE r.id=$1"
            ).bind(setup.run.id).fetch_one(pool).await?;
            assert_eq!(
                state,
                (
                    "stop_requested".into(),
                    "active".into(),
                    "cancelled".into(),
                    true,
                    i64::try_from(setup.run.lease_fencing_token)?,
                    i64::try_from(setup.run.environment_epoch)?
                )
            );
        }
    }
    let snapshot = setup.fixture.snapshot().await?;
    let stops = snapshot
        .rows("event_log")
        .iter()
        .filter(|event| event["event_type"] == "run_stop_requested")
        .collect::<Vec<_>>();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["payload"]["run_id"], json!(setup.run.id));
    assert_eq!(
        stops[0]["payload"]["lease_fencing_token"],
        json!(setup.run.lease_fencing_token)
    );
    assert_eq!(
        stops[0]["payload"]["environment_epoch"],
        json!(setup.run.environment_epoch)
    );
    Ok(())
}
