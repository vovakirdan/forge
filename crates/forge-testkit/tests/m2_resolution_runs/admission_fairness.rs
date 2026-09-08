//! Canonical questions behind a full account still reach a no-Run Human fallback.
use super::*;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic auth and manual Supervisor, no inference"]
async fn account_blocked_page_does_not_hide_later_human_route() -> Result<()> {
    let (h, project, bob, _root) = support::fixture().await?;
    h.store
        .configure_local_admission(forge_domain::admission::AdmissionLimits {
            credential_account_max_runs: 1,
            ..Default::default()
        })
        .await?;
    h.execute(
        project,
        CommandName::AmendEmployee,
        json!({"employee_id":bob,"expected_employee_revision":1,"patch":{"max_concurrent_runs":2}}),
    )
    .await?;
    // An ordinary not-yet-configured resolver exercises the documented Human fallback.
    h.create_employee(project, "Unconfigured lead").await?;
    let unavailable = h
        .store
        .list_employees(project)
        .await?
        .into_iter()
        .find(|employee| employee.employee.name() == "Unconfigured lead")
        .context("unconfigured resolver")?
        .employee
        .id();
    h.execute(
        project,
        CommandName::ConfigureResolverRoute,
        json!({"route_key":"busy_account","employee_ids":[bob],"assignment_timeout_seconds":60}),
    )
    .await?;
    h.execute(project,CommandName::ConfigureResolverRoute,json!({"route_key":"human_fallback","employee_ids":[unavailable],"assignment_timeout_seconds":60})).await?;
    let version = h.create_pipeline(project, single_stage_pipeline()).await?;
    let occupied = h
        .create_task(project, version, "Occupies the shared provider account")
        .await?;
    h.approve_task(project, occupied).await?;
    let task = h
        .create_task(project, version, "Many independent questions on one Task")
        .await?;
    h.approve_task(project, task).await?;
    let mut target = None;
    for index in 0..257 {
        let revision = h
            .store
            .load_task(task)
            .await?
            .context("Question Task")?
            .task
            .revision()
            .get();
        let id:Uuid=h.execute(project,CommandName::RaiseEscalation,json!({"task_id":task,"expected_task_revision":revision,"route_key":if index<256{"busy_account"}else{"human_fallback"},"category":"clarification","question":format!("Independent clarification {index}")})).await?.resource.context("Escalation")?.id.parse()?;
        if index == 256 {
            target = Some(id);
        }
    }
    let target = target.context("last question")?;
    let other = h.create_project("Independent admission cursor").await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    supervisor.next_provision_for_task(occupied).await?;
    // Interleaving another Project must not send the first Project back to page 1.
    h.start_project(other).await?;
    h.core.dispatch_available(project).await?;
    let mut tx = h.store.begin().await?;
    let resolved = tx
        .load_escalation(target)
        .await?
        .context("later question")?;
    let EscalationState::Assigned { assignment_id } = resolved.state else {
        anyhow::bail!("later Human route was starved: {:?}", resolved.state)
    };
    assert_eq!(
        tx.load_resolution_assignment(assignment_id)
            .await?
            .context("Human")?
            .resolver,
        Resolver::Human
    );
    tx.commit().await?;
    let queued:i64=sqlx::query_scalar("SELECT count(*) FROM escalations WHERE project_id=$1 AND escalation_state='queued' AND (canonical_snapshot->>'next_candidate')::integer=0").bind(project.as_uuid()).fetch_one(&h.pool).await?;
    assert_eq!(
        queued, 256,
        "temporary shared-account saturation preserves all Employee routes"
    );
    assert_eq!(
        h.store.list_runs_for_project(project).await?.len(),
        1,
        "Human routing consumes no Run capacity"
    );
    drop(supervisor);
    h.shutdown().await;
    Ok(())
}
