//! Real PostgreSQL admission tests. No provider or Supervisor is started.
use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_domain::{EmployeeId, ProjectId, Timestamp, runtime::RunScope};
use forge_protocol::wire::CommandName;
use forge_storage::{EnvironmentReport, LeaseRunRequest, PostgresStore, RunProjection};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    employees,
    fixture::{Backend, BackendKind, Fixture},
};

async fn setup(capacity: u16, tasks: usize) -> Result<(Fixture, EmployeeId)> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let employee = employees::create(&fixture, "Concurrent Bob").await?;
    if capacity != 1 {
        fixture
            .execute(
                CommandName::AmendEmployee,
                json!({
                    "employee_id":employee,"expected_employee_revision":1,
                    "patch":{"max_concurrent_runs":capacity}
                }),
            )
            .await?;
    }
    let pipeline = fixture
        .pipeline(forge_testkit::m0::single_stage_pipeline())
        .await?;
    for index in 0..tasks {
        let task = fixture
            .create_task(pipeline, &format!("Independent task {index}"))
            .await?;
        fixture
            .task_command(CommandName::ApproveTask, task, json!({}))
            .await?;
    }
    fixture
        .execute(CommandName::StartProjectExecution, json!({}))
        .await?;
    Ok((fixture, employee))
}

fn store(fixture: &Fixture) -> PostgresStore {
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    PostgresStore::from_pool(pool.clone())
}

async fn provision(fixture: &Fixture, employee: EmployeeId) -> Result<Option<RunProjection>> {
    let store = store(fixture);
    let mut tx = store.begin().await?;
    let Some(queue) = tx.claim_next(fixture.project_id).await? else {
        return Ok(None);
    };
    if !tx
        .lock_available_employees(fixture.project_id)
        .await?
        .iter()
        .any(|e| e.employee.id() == employee)
    {
        tx.release_queue_claim(queue.id).await?;
        tx.commit().await?;
        return Ok(None);
    }
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    let expires_at: time::OffsetDateTime =
        sqlx::query_scalar("SELECT clock_timestamp() + interval '1 hour'")
            .fetch_one(pool)
            .await?;
    let surface = Uuid::now_v7();
    let request = LeaseRunRequest {
        queue_entry: queue,
        employee_id: employee,
        lease_id: Uuid::now_v7(),
        run_id: Uuid::now_v7(),
        lease_expires_at: Timestamp::from_offset_date_time(expires_at),
        task_work_surface_id: Some(surface),
        lease_scope: json!({"fixture":true}),
        resource_reservation: json!({}),
        run_spec_version: 1,
        run_spec: json!({"fixture":true}),
        context_manifest: json!({}),
    };
    let run = tx.create_lease_and_run(&request).await?.run;
    tx.reserve_environment(run.id, Some(surface)).await?;
    tx.commit().await?;
    Ok(Some(run))
}

async fn release(fixture: &Fixture, run: &RunProjection, quiescent: bool) -> Result<()> {
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    // Synthetic observation setup: revoke logical authority before proving exit.
    sqlx::query("UPDATE leases SET lease_state='revoked',revoked_at=clock_timestamp() WHERE id=$1")
        .bind(run.lease_id)
        .execute(pool)
        .await?;
    let store = store(fixture);
    let mut tx = store.begin().await?;
    tx.record_environment_report(&EnvironmentReport {
        scope: RunScope {
            run_id: run.id,
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        },
        host_id: "fixture-host".into(),
        boot_id: "fixture-boot".into(),
        environment_id: run.id.to_string(),
        quiescent,
        unknown: !quiescent,
    })
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn read_operations(
    router: &Router,
    project: ProjectId,
    employee: EmployeeId,
) -> Result<(StatusCode, serde_json::Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{project}/employees/{employee}/operations"
                ))
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    let body = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    Ok((status, body))
}

async fn read_resources(
    router: &Router,
    project: ProjectId,
) -> Result<(StatusCode, serde_json::Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/projects/{project}/resources"))
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    let body = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    Ok((status, body))
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates admission resource read"]
async fn project_resources_count_one_run_once_until_physical_release() -> Result<()> {
    let (fixture, employee) = setup(1, 1).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    let router = forge_core::router(fixture.core(pool));
    let (status, initial) = read_resources(&router, fixture.project_id).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["project_id"], json!(fixture.project_id));
    assert_eq!(initial["policy"]["host_max_runs"], 16);
    assert_eq!(initial["policy"]["project_max_runs"], 8);
    assert_eq!(initial["policy"]["credential_account_max_runs"], 4);
    assert!(initial["policy"]["revision"].as_u64().is_some());
    assert!(initial["policy"]["updated_at"].as_str().is_some());
    assert!(initial["observed_at"].as_str().is_some());
    assert_eq!(
        initial["occupancy"],
        json!({
            "host_runs": 0, "project_runs": 0, "credential_account_runs": null
        })
    );

    let run = provision(&fixture, employee)
        .await?
        .context("reserved run")?;
    let (_, occupied) = read_resources(&router, fixture.project_id).await?;
    assert_eq!(occupied["occupancy"]["host_runs"], 1);
    assert_eq!(occupied["occupancy"]["project_runs"], 1);
    release(&fixture, &run, false).await?;
    let (_, uncertain) = read_resources(&router, fixture.project_id).await?;
    assert_eq!(uncertain["occupancy"]["project_runs"], 1);
    release(&fixture, &run, true).await?;
    let (_, released) = read_resources(&router, fixture.project_id).await?;
    assert_eq!(released["occupancy"]["host_runs"], 0);
    assert_eq!(released["occupancy"]["project_runs"], 0);

    let (foreign_status, _) = read_resources(&router, ProjectId::new()).await?;
    assert_eq!(foreign_status, StatusCode::NOT_FOUND);
    let serialized = serde_json::to_string(&occupied)?;
    for forbidden in ["secret_id", "run_spec", "credential_binding", "password"] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates scoped physical occupancy read"]
async fn employee_operations_keep_uncertain_occupancy_and_scope() -> Result<()> {
    let (fixture, employee) = setup(1, 1).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    let router = forge_core::router(fixture.core(pool));
    let (status, initial) = read_operations(&router, fixture.project_id, employee).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["employee_id"], json!(employee));
    assert_eq!(initial["max_concurrent_runs"], 1);
    assert_eq!(initial["occupied_slots"], 0);
    assert_eq!(initial["observed_running_runs"], 0);
    assert_eq!(initial["availability"], "unknown");

    let run = provision(&fixture, employee)
        .await?
        .context("reserved run")?;
    let (_, reserved) = read_operations(&router, fixture.project_id, employee).await?;
    assert_eq!(reserved["occupied_slots"], 1);
    assert_eq!(reserved["observed_running_runs"], 0);

    release(&fixture, &run, false).await?;
    let (_, uncertain) = read_operations(&router, fixture.project_id, employee).await?;
    assert_eq!(uncertain["occupied_slots"], 1);
    release(&fixture, &run, true).await?;
    let (_, released) = read_operations(&router, fixture.project_id, employee).await?;
    assert_eq!(released["occupied_slots"], 0);

    let (foreign_status, _) = read_operations(&router, ProjectId::new(), employee).await?;
    assert_eq!(foreign_status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; run just test-integration"]
async fn employee_capacity_retains_uncertain_environments_and_default_is_one() -> Result<()> {
    let (fixture, employee) = setup(1, 2).await?;
    let first = provision(&fixture, employee).await?.context("first slot")?;
    assert!(provision(&fixture, employee).await?.is_none());
    release(&fixture, &first, false).await?;
    assert!(
        provision(&fixture, employee).await?.is_none(),
        "revocation is not physical exit"
    );
    release(&fixture, &first, true).await?;
    let second = provision(&fixture, employee).await?.context("freed slot")?;
    assert_ne!(first.task_id(), second.task_id());
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; run just test-integration"]
async fn employee_capacity_allows_two_tasks_and_rejects_a_racing_third() -> Result<()> {
    let (fixture, employee) = setup(2, 3).await?;
    let first = provision(&fixture, employee).await?.context("first slot")?;
    let (left, right) = tokio::join!(provision(&fixture, employee), provision(&fixture, employee));
    let second: Vec<_> = [left?, right?].into_iter().flatten().collect();
    assert_eq!(second.len(), 1, "the last capacity slot has one winner");
    assert_ne!(first.task_id(), second[0].task_id());
    assert_ne!(first.id, second[0].id);
    assert!(provision(&fixture, employee).await?.is_none());
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    let occupied: i64 = sqlx::query_scalar("SELECT count(*) FROM run_environment_reservations WHERE employee_id=$1 AND released_at IS NULL")
        .bind(employee.as_uuid()).fetch_one(pool).await?;
    assert_eq!(occupied, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; run just test-integration"]
async fn employee_disable_and_capacity_reduction_never_stop_existing_runs() -> Result<()> {
    let (fixture, employee) = setup(2, 3).await?;
    let first = provision(&fixture, employee).await?.context("first")?;
    let second = provision(&fixture, employee).await?.context("second")?;
    let before = fixture.snapshot().await?.raw;
    fixture
        .execute(
            CommandName::AmendEmployee,
            json!({"employee_id":employee,
        "expected_employee_revision":2,"patch":{"max_concurrent_runs":1}}),
        )
        .await?;
    assert!(provision(&fixture, employee).await?.is_none());
    fixture
        .execute(
            CommandName::DisableEmployee,
            json!({"employee_id":employee,"expected_employee_revision":3}),
        )
        .await?;
    assert!(provision(&fixture, employee).await?.is_none());
    let after = fixture.snapshot().await?.raw;
    for table in ["runs", "leases", "run_environment_reservations"] {
        assert_eq!(
            before[table], after[table],
            "catalog mutation changed physical ownership in {table}"
        );
    }
    release(&fixture, &first, true).await?;
    release(&fixture, &second, true).await?;
    assert!(
        provision(&fixture, employee).await?.is_none(),
        "disabled worker cannot use free capacity"
    );
    Ok(())
}
