//! Onboarding gates real dispatch composition, with synthetic auth and no execution.
#[path = "m3_system_job_ownership/support.rs"]
#[allow(dead_code)]
mod ownership;
#[path = "m2_communication_runs/support.rs"]
#[allow(dead_code)]
mod support;

use anyhow::{Context, Result};
use forge_domain::{EmployeeId, ProjectId};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::json;
use uuid::Uuid;

async fn pending_employee(
    h: &M0Harness,
    project: ProjectId,
    template: EmployeeId,
) -> Result<EmployeeId> {
    let employee: EmployeeId = h
        .execute(
            project,
            CommandName::CreateEmployee,
            json!({"name":"Pending newcomer","role":"resolver","stage_eligibility":{"mode":"any"}}),
        )
        .await?
        .resource
        .context("Employee")?
        .id
        .parse()?;
    let mut tx = h.store.begin().await?;
    let binding = tx
        .runtime_binding(template)
        .await?
        .context("template binding")?;
    assert!(!tx.onboarding_allowed(project, employee).await?);
    assert!(tx.lock_employee_capacity(project, employee).await?);
    tx.commit().await?;
    h.execute(
        project,
        CommandName::ConfigureEmployeeRuntime,
        json!({"employee_id":employee,"binding":binding}),
    )
    .await?;
    Ok(employee)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic credentials, non-executing Supervisor"]
async fn pending_inbox_owner_does_not_block_next_onboarded_owner() -> Result<()> {
    let (h, project, allowed, _root) = support::fixture().await?;
    let pending = pending_employee(&h, project, allowed).await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Draft inbox context")
        .await?;
    let pending_message = support::send_question(&h, project, pending, task).await?;
    support::send_question(&h, project, allowed, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let run = supervisor.next_communication_provision(allowed).await?;
    assert_eq!(run.employee_id, allowed.to_string());
    let runs = h.store.list_runs_for_project(project).await?;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].employee_id, Some(allowed));
    let pending_state: String = sqlx::query_scalar(
        "SELECT state FROM communication_assignments WHERE source_message_id=$1",
    )
    .bind(pending_message)
    .fetch_one(&h.pool)
    .await?;
    assert_eq!(pending_state, "queued");
    h.execute(
        project,
        CommandName::SkipEmployeeOnboarding,
        json!({"employee_id":pending,"reason":"Explicit operator orientation"}),
    )
    .await?;
    h.core.dispatch_available(project).await?;
    assert_eq!(
        supervisor
            .next_communication_provision(pending)
            .await?
            .employee_id,
        pending.to_string()
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic credentials, non-executing Supervisor"]
async fn resolver_route_skips_pending_employee_and_keeps_allowed_candidate() -> Result<()> {
    let (h, project, allowed, _root) = support::fixture().await?;
    let pending = pending_employee(&h, project, allowed).await?;
    let pipeline = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h
        .create_task(project, pipeline, "Question with gated resolver")
        .await?;
    h.approve_task(project, task).await?;
    h.execute(project, CommandName::ConfigureResolverRoute,
        json!({"route_key":"onboarding_route","employee_ids":[pending,allowed],"assignment_timeout_seconds":60})).await?;
    let revision = h
        .store
        .load_task(task)
        .await?
        .context("Task")?
        .task
        .revision()
        .get();
    let escalation: Uuid = h.execute(project, CommandName::RaiseEscalation,
        json!({"task_id":task,"expected_task_revision":revision,"route_key":"onboarding_route","category":"technical_decision","question":"Which published option should be selected?"}))
        .await?.resource.context("Escalation")?.id.parse()?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    let provision = supervisor.next_provision().await?;
    assert_eq!(provision.run_spec_version, 4);
    assert_eq!(provision.employee_id, allowed.to_string());
    let runs = h.store.list_runs_for_project(project).await?;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0]
            .assignment
            .resolution()
            .context("Resolution owner")?
            .escalation_id,
        escalation
    );
    assert_eq!(runs[0].employee_id, Some(allowed));
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn retry_of_skipped_onboarding_clears_receipt_and_closes_gate_atomically() -> Result<()> {
    let f = ownership::setup().await?;
    f.h.execute(
        f.project,
        CommandName::RequestEmployeeOnboarding,
        json!({"employee_id":f.employee}),
    )
    .await?;
    f.h.execute(
        f.project,
        CommandName::SkipEmployeeOnboarding,
        json!({"employee_id":f.employee,"reason":"An explicit, reversible skip"}),
    )
    .await?;
    let skipped = f.h.store.system_job_status(f.project).await?;
    assert_eq!(skipped["jobs"][0]["state"], "cancelled");
    assert_eq!(skipped["onboarding"][0]["state"], "skipped");
    f.h.execute(
        f.project,
        CommandName::RetrySystemJob,
        json!({"job_id":f.spec.assignment.job_id}),
    )
    .await?;
    let retried = f.h.store.system_job_status(f.project).await?;
    assert_eq!(retried["jobs"][0]["state"], "pending");
    assert_eq!(retried["onboarding"][0]["state"], "pending");
    assert!(retried["onboarding"][0]["receipt"].is_null());
    let mut tx = f.h.store.begin().await?;
    assert!(!tx.onboarding_allowed(f.project, f.employee).await?);
    assert!(tx.lock_employee_capacity(f.project, f.employee).await?);
    tx.commit().await?;
    f.h.shutdown().await;
    Ok(())
}
