//! Private-schema PostgreSQL regressions. No provider, credentials or containers.
#[path = "m3_system_job_ownership/support.rs"]
mod support;

use anyhow::{Context, Result};
use forge_domain::{ExecutionProfileInput, runtime::RunScope};
use forge_storage::EnvironmentReport;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; private schema and no provider inference"]
async fn schema_seven_preserves_ownerless_fences_and_immutable_attempt() -> Result<()> {
    let f = support::setup().await?;
    let run = support::admit(&f, &f.spec).await?;
    assert!(run.employee_id.is_none() && run.task_id().is_none());
    assert_eq!(run.assignment.system_job(), Some(&f.spec.assignment));
    let matches: (bool, bool, bool, bool) = sqlx::query_as("SELECT forge_lease_owns_run(l,r),forge_lease_owns_run(jsonb_populate_record(l,jsonb_build_object('system_job_attempt_id',$2::uuid)),r),forge_lease_owns_run(jsonb_populate_record(l,jsonb_build_object('fencing_token',l.fencing_token+1)),r),forge_lease_owns_run(jsonb_populate_record(l,jsonb_build_object('environment_epoch',l.environment_epoch+1)),r) FROM runs r JOIN leases l ON l.id=r.lease_id WHERE r.id=$1")
        .bind(run.id).bind(Uuid::now_v7()).fetch_one(&f.h.pool).await?;
    assert_eq!(matches, (true, false, false, false));
    for sql in [
        "UPDATE run_environment_reservations SET fencing_token=fencing_token+1 WHERE run_id=$1",
        "UPDATE run_environment_reservations SET environment_epoch=environment_epoch+1 WHERE run_id=$1",
    ] {
        let error = sqlx::query(sql)
            .bind(run.id)
            .execute(&f.h.pool)
            .await
            .expect_err("changed physical identity accepted");
        assert_eq!(
            error.as_database_error().and_then(|e| e.constraint()),
            Some("environment_runtime_owner_fk")
        );
    }
    let error = sqlx::query("UPDATE runs SET employee_id=$2 WHERE id=$1")
        .bind(run.id)
        .bind(f.employee.as_uuid())
        .execute(&f.h.pool)
        .await
        .expect_err("immutable SystemJob Run acquired Employee ownership");
    assert!(
        error
            .as_database_error()
            .is_some_and(|error| error.message().contains("run identity"))
    );
    let error =
        sqlx::query("UPDATE run_environment_reservations SET employee_id=$2 WHERE run_id=$1")
            .bind(run.id)
            .bind(f.employee.as_uuid())
            .execute(&f.h.pool)
            .await
            .expect_err("SystemJob reservation acquired Employee ownership");
    assert_eq!(
        error.as_database_error().and_then(|e| e.constraint()),
        Some("environment_employee_purpose_exact")
    );
    for sql in [
        "UPDATE system_job_attempts SET generation=generation+1 WHERE id=$1",
        "UPDATE system_job_attempts SET spec=jsonb_set(spec,'{instruction}','\"changed\"') WHERE id=$1",
        "DELETE FROM system_job_attempts WHERE id=$1",
    ] {
        assert!(
            sqlx::query(sql)
                .bind(f.spec.assignment.attempt_id)
                .execute(&f.h.pool)
                .await
                .is_err()
        );
    }
    assert!(
        sqlx::query(
            "UPDATE runs SET run_spec=jsonb_set(run_spec,'{instruction}','\"changed\"') WHERE id=$1"
        )
        .bind(run.id)
        .execute(&f.h.pool)
        .await
        .is_err()
    );
    let mut tx = f.h.store.begin().await?;
    assert!(
        tx.lock_employee_capacity(f.project, f.employee).await?,
        "context target takes no Employee slot"
    );
    tx.commit().await?;
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; private schema and no provider inference"]
async fn admission_rejects_unapproved_account_input_digest_and_policy_expansion() -> Result<()> {
    let f = support::setup().await?;
    let mut foreign_account = f.spec.clone();
    let mut profile: ExecutionProfileInput =
        foreign_account.binding.execution_profile.clone().into();
    profile.credential_binding.account_id = Some("different-account".into());
    foreign_account.binding.execution_profile = profile.try_into()?;
    let mut oversized_input = f.spec.clone();
    oversized_input.input.context =
        json!({"text":"x".repeat(f.settings.policy.max_input_bytes as usize)});
    let mut wrong_digest = f.spec.clone();
    wrong_digest.input.source_digest = "0".repeat(64);
    let mut wrong_result_bound = f.spec.clone();
    wrong_result_bound.max_result_bytes += 1;
    let mut longer_wall = f.spec.clone();
    longer_wall.binding.limits.wall_seconds += 1;
    for (case, spec) in [
        ("account", foreign_account),
        ("input", oversized_input),
        ("digest", wrong_digest),
        ("result", wrong_result_bound),
        ("wall", longer_wall),
    ] {
        assert!(
            support::admit(&f, &spec).await.is_err(),
            "{case} bypassed approved settings"
        );
    }
    let attempts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM system_job_attempts WHERE project_id=$1")
            .bind(f.project.as_uuid())
            .fetch_one(&f.h.pool)
            .await?;
    assert_eq!(attempts, 0, "invalid specs wrote no canonical attempt");
    support::admit(&f, &f.spec).await?;
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; private schema and no provider inference"]
async fn logical_revoke_retains_semantic_and_shared_capacity_until_physical_quiescence()
-> Result<()> {
    let f = support::setup().await?;
    f.h.store
        .configure_local_admission(forge_domain::admission::AdmissionLimits {
            credential_account_max_runs: 1,
            ..Default::default()
        })
        .await?;
    let run = support::admit(&f, &f.spec).await?;
    let mut tx = f.h.store.begin().await?;
    tx.lock_project(f.project).await?.context("Project")?;
    let job = tx
        .lock_system_job(f.project, f.spec.assignment.job_id)
        .await?
        .context("Job")?;
    assert_eq!(
        tx.system_job_allowance(&job, &f.settings).await?,
        Some("semantic_capacity")
    );
    assert!(
        !tx.lock_run_admission(f.project, Some(&f.spec.binding.execution_profile))
            .await?
    );
    assert!(
        !tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
            .await?,
        "revocation requires explicit force-stop intent"
    );
    tx.request_run_stop(run.id, run.lease_fencing_token, run.environment_epoch, true)
        .await?;
    assert!(
        tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
            .await?
    );
    assert!(
        tx.retire_system_job_attempt(f.spec.assignment.attempt_id, "stopped")
            .await
            .is_err()
    );
    assert_eq!(
        tx.system_job_allowance(&job, &f.settings).await?,
        Some("semantic_capacity")
    );
    assert!(
        !tx.lock_run_admission(f.project, Some(&f.spec.binding.execution_profile))
            .await?
    );
    let scope = RunScope {
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
    };
    let mut report = EnvironmentReport {
        scope,
        host_id: "synthetic-host".into(),
        boot_id: "synthetic-boot".into(),
        environment_id: "synthetic-system-job".into(),
        quiescent: false,
        unknown: true,
    };
    assert!(tx.record_environment_report(&report).await?);
    assert!(
        tx.retire_system_job_attempt(f.spec.assignment.attempt_id, "stopped")
            .await
            .is_err()
    );
    report.unknown = false;
    report.quiescent = true;
    assert!(tx.record_environment_report(&report).await?);
    tx.retire_system_job_attempt(f.spec.assignment.attempt_id, "stopped")
        .await?;
    assert!(
        tx.lock_run_admission(f.project, Some(&f.spec.binding.execution_profile))
            .await?
    );
    assert_eq!(tx.system_job_allowance(&job, &f.settings).await?, None);
    let mut one_attempt = f.settings.clone();
    one_attempt.policy.max_attempts_per_job = 1;
    assert_eq!(
        tx.system_job_allowance(&job, &one_attempt).await?,
        Some("attempt_limit")
    );
    let mut daily = f.settings.clone();
    daily.policy.max_attempts_per_day = 1;
    assert_eq!(
        tx.system_job_allowance(&job, &daily).await?,
        Some("daily_limit")
    );
    tx.commit().await?;
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; private schema and no provider inference"]
async fn onboarding_receipt_cannot_reference_another_projects_job() -> Result<()> {
    let f = support::setup().await?;
    let other = f.h.create_project("Foreign onboarding project").await?;
    f.h.create_employee(other, "Foreign employee").await?;
    let employee =
        f.h.store
            .list_employees(other)
            .await?
            .remove(0)
            .employee
            .id();
    let foreign_job = Uuid::now_v7();
    sqlx::query("INSERT INTO system_jobs(id,project_id,kind,target_employee_id,state,input) VALUES($1,$2,'onboarding',$3,'pending','{}')")
        .bind(foreign_job).bind(other.as_uuid()).bind(employee.as_uuid()).execute(&f.h.pool).await?;
    let error = sqlx::query("UPDATE employee_onboarding SET job_id=$2 WHERE employee_id=$1")
        .bind(f.employee.as_uuid())
        .bind(foreign_job)
        .execute(&f.h.pool)
        .await
        .expect_err("cross-Project onboarding Job accepted");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23503")
    );
    sqlx::query("UPDATE employee_onboarding SET job_id=$2 WHERE employee_id=$1")
        .bind(f.employee.as_uuid())
        .bind(f.spec.assignment.job_id)
        .execute(&f.h.pool)
        .await?;
    f.h.shutdown().await;
    Ok(())
}
