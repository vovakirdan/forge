use anyhow::{Context, Result};
use forge_domain::{EmployeeId, ProjectId, runtime::SystemJobRunSpec, system_job::SystemJobPolicy};
use forge_storage::{RunProjection, SystemJobSettings};
use forge_testkit::m0::M0Harness;
use uuid::Uuid;

#[path = "../../../forge-domain/tests/support/m3_system_job.rs"]
mod fixture;

pub struct Fixture {
    pub h: M0Harness,
    pub project: ProjectId,
    pub employee: EmployeeId,
    pub spec: SystemJobRunSpec,
    pub settings: SystemJobSettings,
}

pub async fn setup() -> Result<Fixture> {
    let h = M0Harness::start_configured(Ok).await?;
    let project = h.create_project("Keyless SystemJob ownership").await?;
    h.create_employee(project, "Context-only employee").await?;
    let employee = h
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    // No queue/settings exist yet, so opening the gate cannot start semantic work.
    h.start_project(project).await?;
    let mut spec = fixture::spec(project, employee);
    spec.binding.limits.wall_seconds = 60;
    spec.binding.limits.stop_grace_seconds = spec.binding.limits.stop_grace_seconds.min(60);
    let settings = SystemJobSettings {
        revision: 1,
        binding: spec.binding.clone(),
        policy: SystemJobPolicy {
            enabled: true,
            wall_seconds: 60,
            ..Default::default()
        },
    };
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("Project")?;
    tx.save_system_job_settings(project, &settings).await?;
    tx.commit().await?;
    sqlx::query("INSERT INTO system_jobs(id,project_id,kind,target_employee_id,state,input) VALUES($1,$2,'onboarding',$3,'pending','{}')")
        .bind(spec.assignment.job_id).bind(project.as_uuid()).bind(employee.as_uuid()).execute(&h.pool).await?;
    Ok(Fixture {
        h,
        project,
        employee,
        spec,
        settings,
    })
}

pub async fn admit(f: &Fixture, spec: &SystemJobRunSpec) -> Result<RunProjection> {
    let mut tx = f.h.store.begin().await?;
    tx.lock_project(f.project).await?.context("Project")?;
    let job = tx
        .lock_system_job(f.project, spec.assignment.job_id)
        .await?
        .context("Job")?;
    let run = tx
        .create_system_job_run(&job, spec, Uuid::now_v7(), Uuid::now_v7())
        .await?;
    tx.commit().await?;
    Ok(run)
}
