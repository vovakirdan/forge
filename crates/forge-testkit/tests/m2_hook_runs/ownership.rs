//! Genuine Hook admission shares the ledger, never the provider owner or grants.
use super::*;
use forge_domain::{EmployeeId, runtime::RunScope};
use forge_storage::{RunDesiredState, RunProjection};

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; controlled Hook observation, no provider"]
async fn shared_admission_hook_consumes_host_but_no_employee_or_account_slot() -> Result<()> {
    let limits = forge_domain::admission::AdmissionLimits {
        host_max_runs: 2,
        project_max_runs: 8,
        credential_account_max_runs: 1,
    };
    let mut fixture = Box::pin(support::setup_with_limits("passed", limits)).await?;
    let provision = fixture.supervisor.next_provision().await?;
    let run = fixture
        .h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Hook")?;
    let employee = fixture
        .h
        .store
        .list_employees(fixture.project)
        .await?
        .remove(0)
        .employee;
    let mut tx = fixture.h.store.begin().await?;
    assert!(
        tx.lock_employee_capacity(fixture.project, employee.id())
            .await?
    );
    let binding = tx
        .runtime_binding(employee.id())
        .await?
        .context("provider binding")?;
    assert!(
        tx.lock_run_admission(fixture.project, Some(&binding.execution_profile))
            .await?,
        "Hook consumes no account slot even when account cap is one"
    );
    tx.commit().await?;
    let pipeline = fixture
        .h
        .create_pipeline(fixture.project, forge_testkit::m0::single_stage_pipeline())
        .await?;
    assert_ne!(
        pipeline,
        run.assignment
            .hook()
            .context("Hook assignment")?
            .pipeline_version_id,
        "creating a second Pipeline must return its own immutable version"
    );
    let task = fixture
        .h
        .create_task(
            fixture.project,
            pipeline,
            "Independent provider work beside Hook",
        )
        .await?;
    fixture.h.approve_task(fixture.project, task).await?;
    fixture.h.core.dispatch_available(fixture.project).await?;
    let provider_runs = fixture.h.store.list_runs(task).await?;
    assert_eq!(provider_runs.len(), 1, "provider Run can coexist with Hook");
    assert_eq!(
        provider_runs[0].desired_state,
        RunDesiredState::ProvisionRequested,
        "provider prep did not remain launchable: {:?}",
        provider_runs[0].observed_state
    );
    fixture.supervisor.next_provision_for_task(task).await?;
    let mut tx = fixture.h.store.begin().await?;
    assert!(
        !tx.lock_run_admission(fixture.project, None).await?,
        "Hook plus provider Run occupy both host slots"
    );
    tx.request_run_stop(run.id, run.lease_fencing_token, run.environment_epoch, true)
        .await?;
    assert!(
        tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
            .await?
    );
    assert!(!tx.lock_run_admission(fixture.project, None).await?);
    tx.commit().await?;
    fixture.supervisor.send_hook_result(&run, 1, None).await?;
    let mut tx = fixture.h.store.begin().await?;
    assert!(tx.lock_run_admission(fixture.project, None).await?);
    tx.commit().await?;
    fixture.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; controlled Hook observations, no provider"]
async fn hook_ownerless_scope_is_fenced_without_employee_capacity_or_gateway() -> Result<()> {
    let mut fixture = Box::pin(support::setup("passed")).await?;
    let provision = fixture.supervisor.next_provision().await?;
    let run = fixture
        .h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Hook")?;
    assert!(run.assignment.hook().is_some());
    assert!(run.employee_id.is_none());
    assert!(run.task_id().is_none());
    assert!(run.require_employee_id().is_err());
    assert!(run.require_task_id().is_err());
    assert!(
        provision.employee_id.is_empty()
            && provision.task_id.is_empty()
            && provision.stage_id.is_empty()
    );
    assert!(
        !fixture
            .root
            .join("gateways")
            .join(run.id.to_string())
            .exists()
    );
    let scope = RunScope {
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
    };
    let employee = fixture
        .h
        .store
        .list_employees(fixture.project)
        .await?
        .remove(0)
        .employee;
    let mut tx = fixture.h.store.begin().await?;
    assert!(tx.validate_gateway_scope(&scope).await?.is_none());
    assert!(
        tx.lock_employee_capacity(fixture.project, employee.id())
            .await?,
        "Hook never occupies an Employee slot"
    );
    assert!(
        tx.lock_active_runs_for_employee(fixture.project, employee.id())
            .await?
            .is_empty()
    );
    assert!(
        tx.lock_active_runs_for_task(fixture.project, fixture.task)
            .await?
            .iter()
            .any(|r| r.id == run.id)
    );
    assert!(
        tx.lock_active_runs_for_project(fixture.project)
            .await?
            .iter()
            .any(|r| r.id == run.id)
    );
    tx.commit().await?;
    sql_scope_guards(&fixture.h, &run, employee.id()).await?;
    let provider_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_credential_snapshots WHERE run_id=$1")
            .bind(run.id)
            .fetch_one(&fixture.h.pool)
            .await?;
    assert_eq!(provider_count, 0);
    assert!(
        fixture
            .h
            .store
            .list_recovery_runs()
            .await?
            .iter()
            .any(|r| r.run_id == run.id)
    );

    fixture.h.execute(fixture.project,CommandName::StopEmployee,
        json!({"employee_id":employee.id(),"expected_employee_revision":employee.revision(),"mode":"force"})).await?;
    assert_eq!(
        fixture
            .h
            .store
            .load_run(run.id)
            .await?
            .context("Hook")?
            .desired_state,
        RunDesiredState::ProvisionRequested
    );
    let revision = fixture
        .h
        .store
        .load_task(fixture.task)
        .await?
        .context("Task")?
        .task
        .revision()
        .get();
    fixture
        .h
        .execute(
            fixture.project,
            CommandName::PauseTask,
            json!({"task_id":fixture.task,"expected_task_revision":revision,"mode":"force"}),
        )
        .await?;
    let stop = fixture
        .supervisor
        .next_stop_for_run(&run.id.to_string())
        .await?;
    assert_eq!(
        stop.mode,
        forge_protocol::supervisor::v1::StopMode::Force as i32
    );
    assert_eq!(
        stop.grace_period_ms, 1000,
        "typed Hook limits govern stop grace"
    );
    let mut tx = fixture.h.store.begin().await?;
    let state = tx
        .run_recovery_state(run.id)
        .await?
        .context("physical reservation")?;
    assert!(
        !state.lease_active && state.reserved,
        "logical revoke never proves process exit"
    );
    assert!(
        tx.lock_active_runs_for_task(fixture.project, fixture.task)
            .await?
            .iter()
            .any(|r| r.id == run.id)
    );
    tx.commit().await?;
    fixture.supervisor.send_hook_result(&run, 1, None).await?;
    let mut tx = fixture.h.store.begin().await?;
    assert!(
        !tx.run_recovery_state(run.id)
            .await?
            .context("physical reservation")?
            .reserved
    );
    tx.commit().await?;
    fixture.h.core.collect_run_evidence(run.id).await?;
    let evidence_streams: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_evidence_streams WHERE run_id=$1")
            .bind(run.id)
            .fetch_one(&fixture.h.pool)
            .await?;
    assert_eq!(
        evidence_streams, 2,
        "provider-free evidence collection needs no credential snapshot"
    );
    assert!(
        !fixture
            .h
            .store
            .evidence_import_candidates(Some(run.id))
            .await?
            .contains(&run.id),
        "absent provider report must not enqueue Hook forever"
    );
    fixture.h.shutdown().await;
    Ok(())
}

async fn sql_scope_guards(
    h: &forge_testkit::m0::M0Harness,
    run: &RunProjection,
    employee: EmployeeId,
) -> Result<()> {
    // MATCH SIMPLE would ignore the old composite FK because Employee is NULL.
    // Each changed physical scope must hit the new all-non-null identity FK.
    for sql in [
        "UPDATE run_environment_reservations SET environment_epoch=environment_epoch+1 WHERE run_id=$1",
        "UPDATE run_environment_reservations SET fencing_token=fencing_token+1 WHERE run_id=$1",
    ] {
        let error = sqlx::query(sql)
            .bind(run.id)
            .execute(&h.pool)
            .await
            .expect_err("mismatched physical fence accepted");
        assert_eq!(
            error.as_database_error().and_then(|e| e.constraint()),
            Some("environment_runtime_owner_fk")
        );
    }
    let error =
        sqlx::query("UPDATE run_environment_reservations SET employee_id=$2 WHERE run_id=$1")
            .bind(run.id)
            .bind(employee.as_uuid())
            .execute(&h.pool)
            .await
            .expect_err("Hook acquired Employee identity");
    assert_eq!(
        error.as_database_error().and_then(|e| e.constraint()),
        Some("environment_employee_purpose_exact")
    );
    let error=sqlx::query("INSERT INTO leases(id,project_id,task_id,queue_entry_id,employee_id,expires_at,purpose) SELECT $1,project_id,task_id,queue_entry_id,NULL,clock_timestamp()+interval '1 hour',purpose FROM leases WHERE project_id=$2 AND purpose='task_stage' LIMIT 1")
        .bind(Uuid::now_v7()).bind(run.project_id.as_uuid()).execute(&h.pool).await.expect_err("provider-free TaskStage admitted");
    assert_eq!(
        error.as_database_error().and_then(|e| e.constraint()),
        Some("leases_employee_purpose_exact")
    );
    let foreign = h.create_project("Foreign ownerless scope").await?;
    assert!(
        sqlx::query("UPDATE run_environment_reservations SET project_id=$2 WHERE run_id=$1")
            .bind(run.id)
            .bind(foreign.as_uuid())
            .execute(&h.pool)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; controlled Hook observations, no provider"]
async fn project_breaker_stops_ownerless_hook_with_retained_physical_scope() -> Result<()> {
    let mut fixture = Box::pin(support::setup("passed")).await?;
    let provision = fixture.supervisor.next_provision().await?;
    let run = fixture
        .h
        .store
        .load_run(provision.run_id.parse()?)
        .await?
        .context("Hook")?;
    fixture.h.stop_project(fixture.project).await?;
    let stop = fixture
        .supervisor
        .next_stop_for_run(&run.id.to_string())
        .await?;
    assert_eq!(
        stop.mode,
        forge_protocol::supervisor::v1::StopMode::Graceful as i32
    );
    let mut tx = fixture.h.store.begin().await?;
    let state = tx
        .run_recovery_state(run.id)
        .await?
        .context("Hook reservation")?;
    assert!(state.reserved, "breaker request is not physical completion");
    assert_eq!(
        tx.load_run(run.id).await?.context("Hook")?.desired_state,
        RunDesiredState::StopRequested
    );
    tx.commit().await?;
    fixture.supervisor.send_hook_result(&run, 1, None).await?;
    fixture.h.shutdown().await;
    Ok(())
}
