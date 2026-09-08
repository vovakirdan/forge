//! Real canonical admissions, synthetic credentials, no provider inference.
use super::*;
use forge_domain::{
    EmployeeId, ProjectId, TaskId, admission::AdmissionLimits, runtime::RuntimeBinding,
};
use forge_testkit::m0::M0Harness;
use std::path::Path;

async fn limits(h: &M0Harness, host: u16, project: u16, account: u16) -> Result<()> {
    h.core
        .clone()
        .with_admission_limits(AdmissionLimits {
            host_max_runs: host,
            project_max_runs: project,
            credential_account_max_runs: account,
        })
        .await?;
    Ok(())
}

async fn queued_task(h: &M0Harness, project: ProjectId, name: &str) -> Result<TaskId> {
    let version = h.create_pipeline(project, single_stage_pipeline()).await?;
    let task = h.create_task(project, version, name).await?;
    h.approve_task(project, task).await?;
    Ok(task)
}

async fn another_project(
    h: &M0Harness,
    source: EmployeeId,
    root: &Path,
    account: &str,
) -> Result<(ProjectId, EmployeeId)> {
    let project = h.create_project("Independent admission Project").await?;
    let employee = add_worker(h, source, root, project, "Other worker", account).await?;
    Ok((project, employee))
}

async fn add_worker(
    h: &M0Harness,
    source: EmployeeId,
    root: &Path,
    project: ProjectId,
    name: &str,
    account: &str,
) -> Result<EmployeeId> {
    h.create_employee(project, name).await?;
    let employee = h
        .store
        .list_employees(project)
        .await?
        .into_iter()
        .find(|value| value.employee.name() == name)
        .context("new Employee")?
        .employee
        .id();
    let mut tx = h.store.begin().await?;
    let mut binding = serde_json::to_value(
        tx.runtime_binding(source)
            .await?
            .context("source binding")?,
    )?;
    tx.commit().await?;
    let secret = Uuid::now_v7();
    let credential = Uuid::now_v7();
    binding["execution_profile"]["id"] = json!(Uuid::now_v7());
    binding["execution_profile"]["project_id"] = json!(project);
    binding["execution_profile"]["credential_binding"] = json!({"id":credential,"project_id":project,"secret_id":secret,"account_id":account,"allowed_delivery_modes":["isolated_runtime_secret"]});
    h.execute(project,CommandName::EnrollCredential,json!({"secret_id":secret,"binding_id":credential,"kind":"claude_subscription","source_file":root.join("synthetic-token")})).await?;
    h.execute(
        project,
        CommandName::ConfigureEmployeeRuntime,
        json!({"employee_id":employee,"binding":binding}),
    )
    .await?;
    Ok(employee)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn shared_admission_pinned_full_account_does_not_starve_later_task() -> Result<()> {
    let (h, p1, bob, root) = fixture().await?;
    limits(&h, 16, 8, 1).await?;
    let (p2, aliased) = another_project(&h, bob, &root, "synthetic-account").await?;
    let zoe = add_worker(&h, bob, &root, p2, "Zoe", "free-account").await?;
    let source = queued_task(&h, p1, "Account consumer").await?;
    let head = queued_task(&h, p2, "Pinned blocked head").await?;
    let pipeline = h
        .store
        .load_task(head)
        .await?
        .context("Task")?
        .task
        .pipeline()
        .pipeline_version_id();
    let later = h
        .create_task(p2, pipeline, "Independent later Task")
        .await?;
    h.approve_task(p2, later).await?;
    for (task, employee) in [(head, aliased), (later, zoe)] {
        let revision = h
            .store
            .load_task(task)
            .await?
            .context("Task")?
            .task
            .revision()
            .get();
        h.execute(
            p2,
            CommandName::SetNextRunEmployee,
            json!({"task_id":task,"expected_task_revision":revision,"employee_id":employee}),
        )
        .await?;
    }
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(p1).await?;
    supervisor.next_provision_for_task(source).await?;
    h.start_project(p2).await?;
    let allowed = supervisor.next_provision_for_task(later).await?;
    assert_eq!(allowed.employee_id, zoe.to_string());
    assert!(h.store.list_runs(head).await?.is_empty());
    let mut tx = h.store.begin().await?;
    assert_eq!(
        tx.load_task_dispatch_constraint(p2, head)
            .await?
            .context("pending")?
            .state,
        forge_domain::NextRunConstraintState::Pending
    );
    tx.commit().await?;
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn shared_admission_serializes_cross_project_host_cap_and_retains_uncertain_run() -> Result<()>
{
    let (h, p1, bob, root) = fixture().await?;
    limits(&h, 1, 8, 4).await?;
    let (p2, _) = another_project(&h, bob, &root, "other-account").await?;
    let t1 = queued_task(&h, p1, "First").await?;
    let t2 = queued_task(&h, p2, "Second").await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let (a, b) = tokio::join!(h.start_project(p1), h.start_project(p2));
    a?;
    b?;
    let provision = supervisor.next_provision().await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("only Run")?;
    assert_eq!(
        h.store.list_runs(t1).await?.len() + h.store.list_runs(t2).await?.len(),
        1,
        "distinct Project transactions share the host cap"
    );
    limits(&h, 1, 8, 4).await?;
    assert!(
        limits(&h, 2, 8, 4).await.is_err(),
        "configuration cannot change under a live or uncertain Run"
    );
    let pending = if run.project_id == p1 { p2 } else { p1 };
    let mut tx = h.store.begin().await?;
    tx.lock_project(run.project_id).await?;
    tx.request_run_stop(run.id, run.lease_fencing_token, run.environment_epoch, true)
        .await?;
    assert!(
        tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
            .await?
    );
    assert!(!tx.lock_run_admission(pending, None).await?);
    tx.commit().await?;
    assert!(
        limits(&h, 2, 8, 4).await.is_err(),
        "revoked Lease is not physical quiescence"
    );
    assert_eq!(h.core.dispatch_available(pending).await?, 0);
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    h.core.dispatch_available(pending).await?;
    let second = supervisor.next_provision().await?;
    assert_ne!(second.run_id, provision.run_id);
    assert_eq!(
        h.store.list_runs(t1).await?.len() + h.store.list_runs(t2).await?.len(),
        2
    );
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn shared_admission_account_aliases_span_projects_but_do_not_block_other_accounts()
-> Result<()> {
    let (h, p1, bob, root) = fixture().await?;
    limits(&h, 16, 8, 1).await?;
    let (p2, aliased) = another_project(&h, bob, &root, "synthetic-account").await?;
    let (p3, _) = another_project(&h, bob, &root, "independent-account").await?;
    let t1 = queued_task(&h, p1, "Consumes shared account").await?;
    let t2 = queued_task(&h, p2, "Different secret, same account").await?;
    let t3 = queued_task(&h, p3, "Independent account").await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(p1).await?;
    let provision = supervisor.next_provision_for_task(t1).await?;
    let run = h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("first Run")?;
    h.start_project(p2).await?;
    h.start_project(p3).await?;
    supervisor.next_provision_for_task(t3).await?;
    assert!(h.store.list_runs(t2).await?.is_empty());
    let mut tx = h.store.begin().await?;
    let aliased_binding = tx.runtime_binding(aliased).await?.context("alias")?;
    assert!(tx.lock_employee_capacity(p2, aliased).await?);
    assert!(
        !tx.lock_run_admission(p2, Some(&aliased_binding.execution_profile))
            .await?
    );
    tx.commit().await?;
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    h.core.dispatch_available(p2).await?;
    supervisor.next_provision_for_task(t2).await?;
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn shared_admission_project_cap_counts_task_and_communication_once_each() -> Result<()> {
    let (h, project, bob, root) = fixture().await?;
    limits(&h, 16, 2, 4).await?;
    let current = h.store.list_employees(project).await?.remove(0).employee;
    h.execute(project,CommandName::AmendEmployee,json!({"employee_id":bob,"expected_employee_revision":current.revision(),"patch":{"max_concurrent_runs":3}})).await?;
    let task = queued_task(&h, project, "Task execution").await?;
    let source = send_question(&h, project, bob, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let communication = supervisor.next_provision().await?;
    let run = h
        .store
        .load_run(communication.run_id.parse()?)
        .await?
        .context("Communication")?;
    assert!(run.assignment.communication().is_some());
    let pipeline = h
        .store
        .load_task(task)
        .await?
        .context("Task")?
        .task
        .pipeline()
        .pipeline_version_id();
    let third = h
        .create_task(project, pipeline, "Must wait for physical capacity")
        .await?;
    h.approve_task(project, third).await?;
    assert_eq!(h.store.list_runs_for_project(project).await?.len(), 2);
    let mut tx = h.store.begin().await?;
    let binding: RuntimeBinding = tx.runtime_binding(bob).await?.context("binding")?;
    assert!(tx.lock_employee_capacity(project, bob).await?);
    assert!(
        !tx.lock_run_admission(project, Some(&binding.execution_profile))
            .await?
    );
    tx.commit().await?;
    let client = client(&root, &run)?;
    assert!(
        call(
            &client,
            "inbox.acknowledge",
            json!({"target_message_id":source}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    assert!(
        call(
            &client,
            "inbox.reply",
            json!({"target_message_id":source,"body":"Synthetic answer"}),
            Uuid::now_v7()
        )
        .await?
        .0
        .is_success()
    );
    assert!(
        call(&client, "communication.complete", json!({}), Uuid::now_v7())
            .await?
            .0
            .is_success()
    );
    assert!(
        h.store.list_runs(third).await?.is_empty(),
        "logical answer does not release the physical slot"
    );
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    h.core.dispatch_available(project).await?;
    supervisor.next_provision_for_task(third).await?;
    h.shutdown().await;
    Ok(())
}
