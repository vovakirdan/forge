//! Explicit future assignment is a separate durable intent, not Task ownership.
use anyhow::{Context, Result};
use forge_application::CommandTransaction;
use forge_domain::{ConstraintEndReason, NextRunConstraintState, NextRunEmployeeConstraint};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::json;
use uuid::Uuid;

use super::{
    active_runs::{RunningTask, running_task},
    atomicity::assert_faults,
    employees,
    fixture::{Backend, BackendKind},
};

pub async fn future_assignment_is_audited_without_moving_current_work(
    kind: BackendKind,
) -> Result<()> {
    let setup = running_task(kind).await?;
    let fixture = &setup.fixture;
    let next = employees::create(fixture, "Next employee").await?;
    let before = fixture.snapshot().await?;
    let command = fixture.envelope(CommandName::SetNextRunEmployee,
        before.projects[&fixture.project_id].revision(),
        json!({"task_id":setup.task,"expected_task_revision":fixture.task(setup.task).await?.revision().get(),"employee_id":next,"reason":"Continue on a fresh worker"}),
        "future-assignment");
    assert_faults(fixture, &command).await?;
    fixture.execute_as(&command, &fixture.context).await?;
    assert_eq!(
        fixture.execute_as(&command, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    let constraint = active(&setup).await?.context("pending constraint")?;
    let task = fixture.task(setup.task).await?;
    assert!(constraint.applies_to(&task));
    assert_eq!(constraint.employee_id, next);
    let after = fixture.snapshot().await?;
    assert_eq!(
        before.tasks, after.tasks,
        "future assignment is not Task mutation"
    );
    assert_eq!(before.raw["runs"], after.raw["runs"]);
    assert_eq!(before.raw["queue_entries"], after.raw["queue_entries"]);
    fixture
        .task_command(
            CommandName::SetNextRunEmployee,
            setup.task,
            json!({"employee_id":setup.run.employee_id}),
        )
        .await?;
    let replacement = active(&setup).await?.context("replacement")?;
    assert_ne!(replacement.id, constraint.id);
    assert_eq!(replacement.employee_id, setup.run.require_employee_id()?);
    fixture
        .task_command(
            CommandName::ClearNextRunEmployee,
            setup.task,
            json!({"reason":"Return to ordinary admission"}),
        )
        .await?;
    assert!(active(&setup).await?.is_none());
    let snapshot = fixture.snapshot().await?;
    snapshot.assert_audit_atomic();
    let history: Vec<_> = snapshot
        .rows("event_log")
        .iter()
        .filter(|row| row["event_type"] == "task_dispatch_constraint_changed")
        .collect();
    assert_eq!(
        history.len(),
        4,
        "set, supersede, replacement, clear are separate facts"
    );
    assert_eq!(snapshot.rows("task_next_run_constraints").len(), 2);
    Ok(())
}

pub async fn future_assignment_refusals_preserve_state(kind: BackendKind) -> Result<()> {
    let setup = running_task(kind).await?;
    let fixture = &setup.fixture;
    fixture
        .task_command(
            CommandName::SetNextRunEmployee,
            setup.task,
            json!({"employee_id":setup.run.employee_id}),
        )
        .await?;
    let revision = fixture.task(setup.task).await?.revision().get();
    for payload in [
        json!({"task_id":setup.task,"expected_task_revision":revision-1,"employee_id":setup.run.employee_id}),
        json!({"task_id":setup.task,"expected_task_revision":revision,"employee_id":Uuid::now_v7()}),
        json!({"task_id":setup.blocker,"expected_task_revision":1,"employee_id":setup.run.employee_id}),
    ] {
        let envelope = fixture.envelope(
            CommandName::SetNextRunEmployee,
            fixture.snapshot().await?.projects[&fixture.project_id].revision(),
            payload,
            &Uuid::now_v7().to_string(),
        );
        fixture
            .assert_unchanged_after(&envelope, &fixture.context)
            .await?;
    }
    fixture
        .execute(
            CommandName::DisableEmployee,
            json!({"employee_id":setup.run.employee_id,"expected_employee_revision":1}),
        )
        .await?;
    let envelope = fixture.envelope(CommandName::SetNextRunEmployee,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"task_id":setup.task,"expected_task_revision":revision,"employee_id":setup.run.employee_id}),"disabled-target");
    fixture
        .assert_unchanged_after(&envelope, &fixture.context)
        .await?;
    let pending = active(&setup)
        .await?
        .context("pending constraint retained")?;
    let mut forged = pending.clone();
    forged.transition(NextRunConstraintState::Consumed {
        run_id: Uuid::now_v7(),
    })?;
    let before = fixture.snapshot().await?.raw;
    match &fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            assert!(
                tx.update_task_dispatch_constraint(&forged, &pending.state)
                    .await
                    .is_err()
            );
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            assert!(
                tx.update_task_dispatch_constraint(&forged, &pending.state)
                    .await
                    .is_err()
            );
        }
    }
    assert_eq!(before, fixture.snapshot().await?.raw);
    let mut invalid = pending.clone();
    invalid.transition(NextRunConstraintState::Cancelled {
        reason: ConstraintEndReason::Cleared,
    })?;
    assert!(invalid.transition(NextRunConstraintState::Pending).is_err());
    Ok(())
}

async fn active(setup: &RunningTask) -> Result<Option<NextRunEmployeeConstraint>> {
    match &setup.fixture.backend {
        Backend::Memory(store) => {
            let mut tx = store.begin().await;
            Ok(tx
                .load_task_dispatch_constraint(setup.fixture.project_id, setup.task)
                .await?)
        }
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            let mut tx = store.begin().await?;
            Ok(tx
                .load_task_dispatch_constraint(setup.fixture.project_id, setup.task)
                .await?)
        }
    }
}
