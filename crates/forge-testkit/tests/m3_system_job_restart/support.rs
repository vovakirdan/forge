use anyhow::{Context, Result};
use forge_domain::{runtime::SurfaceSpec, system_job::SystemJobPolicy};
use forge_protocol::wire::CommandName;
use forge_provider_common::{SealedSecret, SecretScope, SecretStore};
use forge_storage::{PostgresStore, RunProjection};
use forge_testkit::m0::M0Harness;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

#[path = "../m2_communication_runs/support.rs"]
#[allow(dead_code)]
mod runtime;
#[path = "../../../forge-domain/tests/support/m3_system_job.rs"]
mod specification;

pub struct Fixture {
    pub h: M0Harness,
    pub run: RunProjection,
    pub root: PathBuf,
}

pub async fn setup() -> Result<Fixture> {
    let (h, project, employee, root) = runtime::fixture_mode(true).await?;
    let mut tx = h.store.begin().await?;
    let mut binding = tx
        .runtime_binding(employee)
        .await?
        .context("employee binding")?;
    tx.commit().await?;
    binding.surface = SurfaceSpec::None;
    binding.limits.wall_seconds = 60;
    binding.limits.stop_grace_seconds = binding.limits.stop_grace_seconds.min(60);
    let policy = SystemJobPolicy {
        enabled: true,
        coalesce_seconds: 0,
        wall_seconds: 60,
        ..Default::default()
    };
    h.execute(
        project,
        CommandName::ConfigureSystemJobs,
        json!({
            "expected_settings_revision":0,"policy":policy,"binding":binding
        }),
    )
    .await?;
    h.execute(
        project,
        CommandName::RequestEmployeeOnboarding,
        json!({"employee_id":employee}),
    )
    .await?;
    // Admission is seeded before attaching a Supervisor. The old Core owns no
    // Gateway listener, so shutting it down releases the execution-root lock.
    h.start_project(project).await?;
    let job = h.store.system_jobs(project).await?.remove(0);
    let mut spec = specification::spec(project, employee);
    spec.assignment.job_id = job.id;
    spec.assignment.generation = job.generation;
    spec.binding = binding;
    let mut tx = h.store.begin().await?;
    tx.lock_project(project).await?.context("Project")?;
    spec.input.context = tx.system_job_source_context(&job).await?;
    spec.input.source_digest = super::digest(&spec.input.context)?;
    spec.input.covered_sequence = job.covered_sequence;
    let credential = tx
        .load_credential(
            project,
            spec.binding
                .execution_profile
                .credential_binding()
                .secret_id,
        )
        .await?
        .context("synthetic enrolled credential")?;
    let run = tx
        .create_system_job_run(&job, &spec, Uuid::now_v7(), Uuid::now_v7())
        .await?;
    tx.pin_run_credential(run.id, &credential).await?;
    tx.commit().await?;
    Ok(Fixture { h, run, root })
}

/// A missing/wrong master key must fail independently of the no-restoration assertion.
pub async fn assert_reopened_credential(
    secrets: &SecretStore,
    store: &PostgresStore,
    run: &RunProjection,
) -> Result<()> {
    let mut tx = store.begin().await?;
    let record = tx
        .run_credential(run.id)
        .await?
        .context("pinned credential")?;
    tx.commit().await?;
    let spec: forge_domain::runtime::SystemJobRunSpec =
        serde_json::from_value(run.run_spec.clone())?;
    let binding = spec.binding.execution_profile.credential_binding();
    assert_eq!(record.sealed_record["kind"], "claude_subscription");
    assert_eq!(record.sealed_record["binding_id"], json!(binding.id));
    let sealed: SealedSecret = serde_json::from_value(record.sealed_record["sealed"].clone())?;
    let expected = SecretScope {
        secret_id: binding.secret_id,
        project_id: run.project_id.as_uuid(),
        version: record.version,
        purpose: format!("claude_subscription:{}", binding.id),
    };
    let plaintext = secrets.open(&sealed, &expected)?;
    forge_provider_claude::validate_setup_token(&plaintext)?;
    let mut wrong_scope = expected;
    wrong_scope.project_id = Uuid::now_v7();
    assert!(
        secrets.open(&sealed, &wrong_scope).is_err(),
        "control must authenticate credential scope"
    );
    Ok(())
}
