//! Named Manager control runs against the same borrowed transaction on both stores.
use anyhow::{Context, Result};
use forge_application::CommandTransaction;
use forge_domain::{Actor, ActorId, EmployeeState, LifecycleStatus, StageVisit};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::json;

use super::{
    active_runs::{RunningTask, running_task, running_task_with_visit},
    atomicity::assert_faults,
    employees,
    fixture::{Backend, BackendKind, Fixture},
};

pub async fn individual_pause_preserves_physical_ownership(kind: BackendKind) -> Result<()> {
    let setup = running_task(kind).await?;
    let fixture = &setup.fixture;
    let task = fixture.task(setup.task).await?;
    let command = fixture.envelope(CommandName::PauseTask,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"task_id":setup.task,"expected_task_revision":task.revision().get(),"mode":"graceful","reason":"Inspect partial work"}), "pause-one");
    assert_faults(fixture, &command).await?;
    let receipt = fixture.execute_as(&command, &fixture.context).await?;
    assert_eq!(
        fixture.execute_as(&command, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    let paused = fixture.task(setup.task).await?;
    assert_eq!(paused.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(
        fixture.task(setup.blocker).await?.lifecycle(),
        LifecycleStatus::Draft
    );
    assert_eq!(
        employees::load(fixture, setup.run.require_employee_id()?)
            .await?
            .state(),
        EmployeeState::Enabled
    );
    assert_stop_retained(&setup, false).await?;
    // Escalating the requested stop cannot downgrade the fence or release ownership.
    fixture
        .task_command(CommandName::PauseTask, setup.task, json!({"mode":"force"}))
        .await?;
    assert_eq!(
        fixture.task(setup.task).await?.wait_conditions().count(),
        paused.wait_conditions().count()
    );
    assert_stop_retained(&setup, true).await?;
    fixture
        .task_command(
            CommandName::PauseTask,
            setup.task,
            json!({"mode":"graceful"}),
        )
        .await?;
    assert_stop_retained(&setup, true).await?;
    assert!(!receipt.event_ids.is_empty());
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn employee_stop_is_visit_scoped(kind: BackendKind) -> Result<()> {
    for visit in [
        None,
        Some(StageVisit::INITIAL),
        Some(serde_json::from_value(json!(2))?),
    ] {
        let setup = running_task_with_visit(kind, visit).await?;
        let fixture = &setup.fixture;
        let before = fixture.task(setup.task).await?;
        let command = fixture.envelope(CommandName::StopEmployee,
            fixture.snapshot().await?.projects[&fixture.project_id].revision(),
            json!({"employee_id":setup.run.employee_id,"expected_employee_revision":1,"mode":"force"}), "stop-employee");
        assert_faults(fixture, &command).await?;
        fixture.execute_as(&command, &fixture.context).await?;
        assert_eq!(
            fixture.execute_as(&command, &fixture.context).await?.status,
            CommandStatus::Replayed
        );
        let employee = employees::load(fixture, setup.run.require_employee_id()?).await?;
        assert_eq!(
            (employee.state(), employee.revision()),
            (EmployeeState::Disabled, 2)
        );
        let after = fixture.task(setup.task).await?;
        if visit == Some(StageVisit::INITIAL) {
            assert_eq!(after.lifecycle(), LifecycleStatus::Waiting);
        } else {
            assert_eq!(
                before, after,
                "unknown or different visit cannot pause current work"
            );
        }
        assert_stop_retained(&setup, true).await?;
        fixture
            .execute(
                CommandName::RetireEmployee,
                json!({"employee_id":setup.run.employee_id,"expected_employee_revision":2}),
            )
            .await?;
        fixture.execute(CommandName::StopEmployee, json!({"employee_id":setup.run.employee_id,"expected_employee_revision":3,"mode":"force"})).await?;
        let retired = employees::load(fixture, setup.run.require_employee_id()?).await?;
        assert_eq!(
            (retired.state(), retired.revision()),
            (EmployeeState::Retired, 3)
        );
        // Logical revocation must not make a still-existing execution disappear.
        match &fixture.backend {
            Backend::Memory(store) => {
                let mut tx = store.begin().await;
                assert_eq!(
                    tx.lock_active_runs_for_employee(
                        fixture.project_id,
                        setup.run.require_employee_id()?
                    )
                    .await?
                    .len(),
                    1
                );
            }
            Backend::Postgres(pool) => {
                let store = PostgresStore::from_pool(pool.clone());
                let mut tx = store.begin().await?;
                assert_eq!(
                    tx.lock_active_runs_for_employee(
                        fixture.project_id,
                        setup.run.require_employee_id()?
                    )
                    .await?
                    .len(),
                    1
                );
            }
        }
    }
    Ok(())
}

pub async fn ready_pause_can_resume_to_queue(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture
        .pipeline(forge_testkit::m0::single_stage_pipeline())
        .await?;
    let task = fixture.create_task(pipeline, "Not started").await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    fixture
        .task_command(CommandName::PauseTask, task, json!({"mode":"graceful"}))
        .await?;
    let paused = fixture.task(task).await?;
    assert_eq!(paused.lifecycle(), LifecycleStatus::Waiting);
    let wait = paused.wait_conditions().next().context("pause wait")?.id();
    fixture
        .task_command(
            CommandName::ResumeTask,
            task,
            json!({"wait_condition_id":wait}),
        )
        .await?;
    let resumed = fixture.task(task).await?;
    assert_eq!(resumed.lifecycle(), LifecycleStatus::Ready);
    let snapshot = fixture.snapshot().await?;
    assert_eq!(
        snapshot
            .rows("queue_entries")
            .iter()
            .filter(|row| row["queue_state"] == "queued")
            .count(),
        1
    );
    Ok(())
}

pub async fn management_refusals_are_atomic(kind: BackendKind) -> Result<()> {
    let setup = running_task(kind).await?;
    let fixture = &setup.fixture;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let task_revision = fixture.task(setup.task).await?.revision().get();
    let mut foreign = fixture.clone();
    foreign.project_id = forge_domain::ProjectId::new();
    foreign.context.project_id = foreign.project_id;
    foreign
        .execute(
            CommandName::CreateProject,
            json!({"name":"Foreign management scope"}),
        )
        .await?;
    let foreign_employee = employees::create(&foreign, "Foreign Bob").await?;
    for (name, payload) in [
        (
            CommandName::StopEmployee,
            json!({"employee_id":foreign_employee,"expected_employee_revision":1,"mode":"force"}),
        ),
        (
            CommandName::StopEmployee,
            json!({"employee_id":setup.run.employee_id,"expected_employee_revision":99,"mode":"force"}),
        ),
        (
            CommandName::StopEmployee,
            json!({"employee_id":forge_domain::EmployeeId::new(),"expected_employee_revision":1,"mode":"force"}),
        ),
        (
            CommandName::PauseTask,
            json!({"task_id":setup.task,"expected_task_revision":task_revision+1,"mode":"force"}),
        ),
        (
            CommandName::PauseTask,
            json!({"task_id":setup.blocker,"expected_task_revision":1,"mode":"force"}),
        ),
    ] {
        let command = fixture.envelope(name, revision, payload, &uuid::Uuid::now_v7().to_string());
        fixture
            .assert_unchanged_after(&command, &fixture.context)
            .await?;
    }
    let command = fixture.envelope(
        CommandName::PauseTask,
        revision,
        json!({"task_id":setup.task,"expected_task_revision":task_revision,"mode":"force"}),
        "unauthorized",
    );
    let mut denied = fixture.context.clone();
    denied.actor = Actor::employee(ActorId::new());
    denied.capabilities.clear();
    fixture.assert_unchanged_after(&command, &denied).await?;
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            assert!(
                !tx.revoke_run_lease(
                    setup.run.id,
                    setup.run.lease_fencing_token + 1,
                    setup.run.environment_epoch
                )
                .await?
            );
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            assert!(
                !tx.revoke_run_lease(
                    setup.run.id,
                    setup.run.lease_fencing_token + 1,
                    setup.run.environment_epoch
                )
                .await?
            );
        }
    }
    Ok(())
}

async fn assert_stop_retained(setup: &RunningTask, force: bool) -> Result<()> {
    match &setup.fixture.backend {
        Backend::Memory(store) => {
            let snapshot = store.snapshot().await;
            let run = snapshot
                .runs
                .iter()
                .find(|run| run.scope.id == setup.run.id)
                .context("Run")?;
            assert_eq!(run.stop_requested, Some(force));
            assert_eq!(run.lease_active, !force);
            assert!(run.reservation_held);
        }
        Backend::Postgres(pool) => {
            let actual: (String, String, bool) = sqlx::query_as("SELECT r.desired_state,l.lease_state,e.released_at IS NULL FROM runs r JOIN leases l ON l.id=r.lease_id JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.id=$1")
                .bind(setup.run.id).fetch_one(pool).await?;
            assert_eq!(
                actual,
                (
                    if force {
                        "force_stop_requested"
                    } else {
                        "stop_requested"
                    }
                    .into(),
                    if force { "revoked" } else { "active" }.into(),
                    true
                )
            );
        }
    }
    Ok(())
}
