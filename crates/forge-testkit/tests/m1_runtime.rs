//! Full Core composition with synthetic auth; no login or paid inference.

use anyhow::{Context, Result};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, LifecycleStatus, ProjectId,
    runtime::{ResourceLimits, RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::{supervisor::v1::RunEventKind, wire::CommandName};
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::json;
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use uuid::Uuid;

#[path = "m1_runtime/input_acceptance.rs"]
mod input_acceptance;
#[path = "m1_runtime/review_regressions.rs"]
mod review_regressions;

fn directory() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("fm1-{}", Uuid::now_v7()));
    fs::create_dir(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
fn auth() -> SecretBytes {
    SecretBytes::new(br#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access-token","refresh_token":"synthetic-refresh-token","id_token":"synthetic-id-token","account_id":"synthetic"},"last_refresh":"2026-09-01T10:00:00Z"}"#.to_vec())
}

async fn fixture() -> Result<(M0Harness, PathBuf, ProjectId)> {
    // Capture safe Core warnings in failing integration output, including
    // asynchronous dispatch failures that a command receipt cannot report.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let root = directory()?;
    let store = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let harness = M0Harness::start_configured(move |core| {
        Ok(core.with_secret_store(store).with_execution_root(shared)?)
    })
    .await?;
    let project = harness.create_project("M1 synthetic runtime").await?;
    Ok((harness, root, project))
}

async fn enroll(
    harness: &M0Harness,
    root: &std::path::Path,
    project: ProjectId,
    image: &str,
) -> Result<()> {
    let secret_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    let source = PrivateMaterialization::create(&root.join("input-auth.json"), &auth())?;
    harness.execute(project,CommandName::EnrollCredential,json!({"secret_id":secret_id,"binding_id":binding_id,"kind":"codex_chatgpt","source_file":source.path()})).await?;
    for name in ["Alice", "Bob"] {
        harness.create_employee(project, name).await?;
    }
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let profile = ExecutionProfileInput {
        id: Uuid::now_v7(),
        revision: 1,
        project_id: project,
        adapter_id: "codex_cli".into(),
        adapter_version: "0.153.2".into(),
        provider_id: "openai".into(),
        model: "fixture-no-inference".into(),
        credential_binding: CredentialBinding {
            id: binding_id,
            project_id: project,
            secret_id,
            account_id: Some("synthetic".into()),
            allowed_delivery_modes: BTreeSet::from([mode]),
        },
        credential_delivery: mode,
        capability_profile: forge_provider_codex::CodexAdapter::capabilities(),
    }
    .try_into()?;
    let binding = RuntimeBinding {
        budget: Default::default(),
        execution_profile: profile,
        image: image.into(),
        surface: SurfaceSpec::FilesystemSandbox,
        access: SurfaceAccess::ReadWrite,
        limits: ResourceLimits::default(),
        system_prompt: "Synthetic test only.".into(),
        employee_prompt: "Report using the scoped Forge Gateway.".into(),
    };
    for employee in harness.store.list_employees(project).await? {
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee.employee.id(),"binding":binding}),
            )
            .await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; uses a manual Supervisor, never calls a provider"]
async fn two_runs_share_auth_but_not_files_and_stop_creates_handoff() -> Result<()> {
    let (harness, root, project) = fixture().await?;
    enroll(
        &harness,
        &root,
        project,
        &format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
    )
    .await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let first = harness.create_task(project, pipeline, "first").await?;
    let second = harness.create_task(project, pipeline, "second").await?;
    harness.approve_task(project, first).await?;
    harness.approve_task(project, second).await?;
    harness.start_project(project).await?;
    let provision_a = supervisor.next_provision_for_task(first).await?;
    let provision_b = supervisor.next_provision_for_task(second).await?;
    let run_a = harness.wait_for_run_count(first, 1).await?.remove(0);
    let run_b = harness.wait_for_run_count(second, 1).await?.remove(0);
    for (provision, run) in [(&provision_a, &run_a), (&provision_b, &run_b)] {
        assert_eq!(provision.run_spec_version, 6);
        let Some(forge_protocol::supervisor::v1::provision_run::Assignment::TaskStage(owner)) =
            &provision.assignment
        else {
            anyhow::bail!("v6 requires explicit TaskStage authority on the Supervisor wire");
        };
        let expected = run.require_task_stage()?;
        assert_eq!(owner.task_id, expected.task_id.to_string());
        assert_eq!(owner.stage_id, expected.stage_id.to_string());
        assert_eq!(owner.queue_entry_id, expected.queue_entry_id.to_string());
    }
    assert_ne!(run_a.run_spec["surface_id"], run_b.run_spec["surface_id"]);
    for run in [&run_a, &run_b] {
        let grant = root
            .join("grants")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string());
        assert_eq!(
            PrivateMaterialization::open(&grant.join("auth.json"))?
                .read(1024 * 1024)?
                .expose(),
            auth().expose()
        );
        assert!(!serde_json::to_string(&run.run_spec)?.contains("synthetic-refresh-token"));
        let invocation: serde_json::Value = serde_json::from_slice(
            PrivateMaterialization::open(&grant.join("invocation.json"))?
                .read(1024 * 1024)?
                .expose(),
        )?;
        assert!(
            invocation["args"]
                .as_array()
                .context("runner args")?
                .windows(2)
                .any(|pair| pair == [json!("--cd"), json!("/workspace/worktree")]),
            "CLI must preserve the Supervisor's actual worktree directory"
        );
    }
    let gateway = root
        .join("gateways")
        .join(run_a.id.to_string())
        .join("gateway.sock");
    let client = reqwest::Client::builder().unix_socket(gateway).build()?;
    let denied=client.post("http://localhost/tools").json(&json!({"message_id":Uuid::now_v7(),"tool":"progress.report","arguments":{"note":"synthetic-refresh-token"}})).send().await?;
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
    // Concurrent auth refreshes are collected independently, then CAS is short.
    for (index, run) in [&run_a, &run_b].iter().enumerate() {
        let mut directory = root.join("runtime");
        for component in [
            run.id.to_string(),
            run.environment_epoch.to_string(),
            "codex-home".into(),
        ] {
            directory = directory.join(component);
            fs::create_dir(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let mut changed: serde_json::Value = serde_json::from_slice(auth().expose())?;
        changed["last_refresh"] = json!("2026-09-02T10:00:00Z");
        changed["tokens"]["refresh_token"] = json!(format!("synthetic-refresh-candidate-{index}"));
        PrivateMaterialization::create(
            &directory.join("auth.json"),
            &SecretBytes::new(serde_json::to_vec(&changed)?),
        )?;
    }
    harness
        .execute(project, CommandName::StopProjectExecution, json!({}))
        .await?;
    for run in [&run_a, &run_b] {
        supervisor.next_stop_for_run(&run.id.to_string()).await?;
        supervisor
            .send_observation_kind(run, run.lease_fencing_token, 1, RunEventKind::Stopped)
            .await?;
    }
    assert_eq!(
        harness.required_task(first).await?.task.lifecycle(),
        LifecycleStatus::Waiting
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_handoffs WHERE run_id=ANY($1)")
        .bind(vec![run_a.id, run_b.id])
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(count, 2);
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM provider_credentials WHERE project_id=$1")
            .bind(project.as_uuid())
            .fetch_all(&harness.pool)
            .await?;
    assert_eq!(versions, vec![2]);
    let recovery: String = sqlx::query_scalar(
        "SELECT recovery::text FROM credential_writeback_conflicts WHERE run_id=$1",
    )
    .bind(run_b.id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(!recovery.contains("synthetic-refresh-candidate"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires rootless Podman fixture image and PostgreSQL/NATS; no real provider"]
async fn real_podman_exit_is_not_a_successful_task_outcome() -> Result<()> {
    let (mut harness, root, project) = fixture().await?;
    let image = tokio::process::Command::new("podman")
        .args([
            "image",
            "inspect",
            "localhost/forge-runner-fixture:m1",
            "--format",
            "{{.Digest}}",
        ])
        .output()
        .await?;
    anyhow::ensure!(image.status.success(), "build Containerfile.fixture first");
    let digest = String::from_utf8(image.stdout)?;
    enroll(
        &harness,
        &root,
        project,
        &format!("localhost/forge-runner-fixture@{}", digest.trim()),
    )
    .await?;
    harness.attach_runtime_supervisor(root.clone()).await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "real sandbox fixture")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if harness.required_task(task).await?.task.lifecycle() == LifecycleStatus::Waiting {
                break Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("wait for physical fixture exit")??;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let released: bool = sqlx::query_scalar(
                "SELECT released_at IS NOT NULL FROM run_environment_reservations WHERE run_id=$1",
            )
            .bind(run.id)
            .fetch_one(&harness.pool)
            .await?;
            if released {
                break Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("wait for positive physical quiescence after Task pause")??;
    let handoff: serde_json::Value =
        sqlx::query_scalar("SELECT body FROM task_handoffs WHERE run_id=$1")
            .bind(run.id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(handoff["outcome"]["kind"], "interrupted");
    harness.core.collect_run_evidence(run.id).await?;
    let diagnostics = harness.store.run_diagnostics(run.id).await?;
    assert!(diagnostics["runtime_report"].is_object());
    assert!(!serde_json::to_string(&diagnostics)?.contains("synthetic-refresh-token"));
    // Retained synthetic container/surface provide inspectable evidence.
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic auth and manual Supervisor"]
async fn unsafe_refreshed_auth_retains_raw_evidence_without_export_or_cleanup() -> Result<()> {
    let (harness, root, project) = fixture().await?;
    enroll(
        &harness,
        &root,
        project,
        &format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
    )
    .await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "unsafe returned auth")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let auth_dir = make_private_descendants(
        &root,
        [
            "runtime".into(),
            run.id.to_string(),
            run.environment_epoch.to_string(),
            "codex-home".into(),
        ],
    )?;
    let mut candidate: serde_json::Value = serde_json::from_slice(auth().expose())?;
    candidate["tokens"]["refresh_token"] = json!("synthetic-new-unsafe-refresh");
    let returned = PrivateMaterialization::create(
        &auth_dir.join("auth.json"),
        &SecretBytes::new(serde_json::to_vec(&candidate)?),
    )?;
    fs::set_permissions(returned.path(), fs::Permissions::from_mode(0o644))?;
    let evidence = make_private_descendants(
        &root,
        [
            "evidence".into(),
            run.id.to_string(),
            run.environment_epoch.to_string(),
        ],
    )?;
    PrivateMaterialization::create(
        &evidence.join("stdout.jsonl"),
        &SecretBytes::new(b"synthetic-new-unsafe-refresh".to_vec()),
    )?;
    PrivateMaterialization::create(&evidence.join("stderr.log"), &SecretBytes::new(Vec::new()))?;
    harness
        .execute(project, CommandName::StopProjectExecution, json!({}))
        .await?;
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    harness.core.collect_run_evidence(run.id).await?;
    let diagnostics = harness.store.run_diagnostics(run.id).await?;
    assert!(
        diagnostics["evidence"]
            .as_array()
            .context("evidence array")?
            .is_empty()
    );
    assert!(
        diagnostics["streams"]
            .as_array()
            .context("streams")?
            .iter()
            .all(|stream| stream["incomplete"] == true)
    );
    assert!(!serde_json::to_string(&diagnostics)?.contains("synthetic-new-unsafe-refresh"));
    assert!(returned.path().exists());
    assert!(
        root.join("grants")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string())
            .join("auth.json")
            .exists()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider execution"]
async fn profile_revision_cannot_be_rebound_to_different_contents() -> Result<()> {
    let (harness, root, project) = fixture().await?;
    enroll(
        &harness,
        &root,
        project,
        &format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
    )
    .await?;
    let employee = harness
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    let mut transaction = harness.store.begin().await?;
    let binding = transaction
        .runtime_binding(employee)
        .await?
        .context("binding")?;
    transaction.commit().await?;
    let mut changed = serde_json::to_value(binding)?;
    changed["execution_profile"]["model"] = json!("different-fixture-model");
    assert!(
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee,"binding":changed})
            )
            .await
            .is_err()
    );
    changed["execution_profile"]["revision"] = json!(2);
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":employee,"binding":changed}),
        )
        .await?;
    Ok(())
}

fn make_private_descendants(
    root: &std::path::Path,
    components: impl IntoIterator<Item = String>,
) -> Result<PathBuf> {
    let mut directory = root.to_path_buf();
    for component in components {
        directory = directory.join(component);
        if !directory.exists() {
            fs::create_dir(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(directory)
}
