//! Synthetic Core composition, not authenticated Claude or native MCP proof.
//! A manual Supervisor never starts an executable or contacts a provider.

use anyhow::{Context, Result};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId,
    runtime::{RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::{runtime::RunnerInvocation, supervisor::v1::RunEventKind, wire::CommandName};
use forge_provider_claude::ClaudeAdapter;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_storage::RunProjection;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const TOKEN: &str = "sk-ant-oat01-synthetic-contract-not-a-real-credential";

fn directory(path: &Path) -> Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    Ok(path.to_owned())
}

fn binding(project_id: ProjectId, secret_id: Uuid, binding_id: Uuid) -> Result<RuntimeBinding> {
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    Ok(RuntimeBinding {
        budget: Default::default(),
        execution_profile: ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id,
            adapter_id: "claude_code_cli".into(),
            adapter_version: "2.1.263".into(),
            provider_id: "anthropic".into(),
            model: "synthetic-model-no-inference".into(),
            credential_binding: CredentialBinding {
                id: binding_id,
                project_id,
                secret_id,
                account_id: Some("operator-selected-account-label".into()),
                allowed_delivery_modes: BTreeSet::from([mode]),
            },
            credential_delivery: mode,
            capability_profile: ClaudeAdapter::capabilities(),
        }
        .try_into()?,
        image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
        surface: SurfaceSpec::FilesystemSandbox,
        access: SurfaceAccess::ReadWrite,
        limits: Default::default(),
        system_prompt: "Synthetic contract only.".into(),
        employee_prompt: "Use scoped Forge Gateway; do not infer a Task outcome from exit.".into(),
    })
}

async fn enroll(
    harness: &M0Harness,
    project: ProjectId,
    path: &Path,
    binding: &RuntimeBinding,
    kind: &str,
) -> Result<()> {
    let reference = binding.execution_profile.credential_binding();
    harness.execute(project, CommandName::EnrollCredential, json!({
        "secret_id":reference.secret_id,"binding_id":reference.id,"kind":kind,"source_file":path,
    })).await?;
    Ok(())
}

fn mock_runtime_evidence(root: &Path, run: &RunProjection) -> Result<PathBuf> {
    let relative = PathBuf::from(run.id.to_string()).join(run.environment_epoch.to_string());
    let runtime = directory(&root.join("runtime").join(&relative).join("claude-home"))?;
    PrivateMaterialization::create(
        &runtime.join("setup-token"),
        &SecretBytes::new(TOKEN.as_bytes().to_vec()),
    )?;
    let evidence = directory(&root.join("evidence").join(relative))?;
    let result = json!({"type":"result","session_id":run.id,"subtype":"success","is_error":false,"usage":{"input_tokens":7,"output_tokens":3}});
    for (name, contents) in [
        ("stdout.jsonl", format!("{result}\n")),
        ("stderr.log", String::new()),
        (
            "exit.json",
            json!({"exit_code":0,"output_incomplete":false,"stop_requested":false}).to_string(),
        ),
    ] {
        PrivateMaterialization::create(
            &evidence.join(name),
            &SecretBytes::new(contents.into_bytes()),
        )?;
    }
    Ok(runtime)
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic token and manual Supervisor, no provider calls"]
async fn claude_subscription_is_distinct_scoped_and_cleaned_only_after_quiescence() -> Result<()> {
    let root = directory(&std::env::temp_dir().join(format!("fm2c-{}", Uuid::now_v7())))?;
    let secrets = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let harness = M0Harness::start_configured(move |core| {
        Ok(core
            .with_secret_store(secrets)
            .with_execution_root(shared)?)
    })
    .await?;
    let project = harness.create_project("Claude synthetic contract").await?;
    let source = PrivateMaterialization::create(
        &root.join("explicit-input-token"),
        &SecretBytes::new(format!("{TOKEN}\n").into_bytes()),
    )?;
    let intended = binding(project, Uuid::now_v7(), Uuid::now_v7())?;
    let wrong_kind = binding(project, Uuid::now_v7(), Uuid::now_v7())?;
    enroll(&harness, project, source.path(), &wrong_kind, "api_key").await?;
    enroll(
        &harness,
        project,
        source.path(),
        &intended,
        "claude_subscription",
    )
    .await?;
    for name in ["Claude Alice", "Claude Bob"] {
        harness.create_employee(project, name).await?;
    }
    let employees = harness.store.list_employees(project).await?;
    assert!(
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employees[0].employee.id(),"binding":wrong_kind})
            )
            .await
            .is_err()
    );
    for employee in &employees {
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employee.employee.id(),"binding":intended}),
            )
            .await?;
    }
    let mut unsupported = serde_json::to_value(&intended)?;
    unsupported["execution_profile"]["capability_profile"]["capabilities"]
        .as_array_mut()
        .context("capabilities")?
        .push(json!("session_resume"));
    assert!(
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":employees[0].employee.id(),"binding":unsupported})
            )
            .await
            .is_err()
    );
    let other_project = harness.create_project("unrelated project").await?;
    harness
        .create_employee(other_project, "unrelated employee")
        .await?;
    let other_employee = harness.store.list_employees(other_project).await?.remove(0);
    let cross_project = binding(
        other_project,
        intended.execution_profile.credential_binding().secret_id,
        intended.execution_profile.credential_binding().id,
    )?;
    assert!(
        harness
            .execute(
                other_project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":other_employee.employee.id(),"binding":cross_project})
            )
            .await
            .is_err()
    );
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let first = harness
        .create_task(project, pipeline, "first Claude task")
        .await?;
    let second = harness
        .create_task(project, pipeline, "second Claude task")
        .await?;
    harness.approve_task(project, first).await?;
    harness.approve_task(project, second).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(first).await?;
    supervisor.next_provision_for_task(second).await?;
    let runs = [
        harness.wait_for_run_count(first, 1).await?.remove(0),
        harness.wait_for_run_count(second, 1).await?.remove(0),
    ];
    assert_ne!(runs[0].id, runs[1].id);
    assert_ne!(
        runs[0].run_spec["surface_id"],
        runs[1].run_spec["surface_id"]
    );
    for run in &runs {
        let grant = root
            .join("grants")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string());
        let token =
            PrivateMaterialization::open(&grant.join("claude-setup-token"))?.read(16 * 1024)?;
        assert_eq!(token.expose(), TOKEN.as_bytes());
        assert!(!grant.join("auth.json").exists());
        assert!(!grant.join("api-key").exists());
        let raw =
            PrivateMaterialization::open(&grant.join("invocation.json"))?.read(1024 * 1024)?;
        assert!(!std::str::from_utf8(raw.expose())?.contains(TOKEN));
        let invocation: RunnerInvocation = serde_json::from_slice(raw.expose())?;
        assert_eq!(invocation.program, "forge-claude-driver");
        assert_eq!(
            invocation.env["CLAUDE_CONFIG_DIR"],
            "/run/forge/claude-home"
        );
        assert!(!invocation.credential_files[0].writeback);
        let stdin = PrivateMaterialization::open(&grant.join("stdin"))?.read(1024 * 1024)?;
        let message: Value = serde_json::from_slice(stdin.expose())?;
        assert_eq!(message["session_id"], run.id.to_string());
        assert_eq!(message["uuid"], run.id.to_string());
        let socket = root
            .join("gateways")
            .join(run.id.to_string())
            .join("gateway.sock");
        let client = reqwest::Client::builder().unix_socket(socket).build()?;
        let response = client.post("http://localhost/tools").json(&json!({"message_id":Uuid::now_v7(),"tool":"progress.report","arguments":{"note":TOKEN}})).send().await?;
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        let runtime = mock_runtime_evidence(&root, run)?;
        harness.core.collect_run_evidence(run.id).await?;
        assert!(
            runtime.join("setup-token").exists(),
            "no cleanup before physical quiescence"
        );
    }
    harness.stop_project(project).await?;
    for run in &runs {
        supervisor.next_stop_for_run(&run.id.to_string()).await?;
        supervisor
            .send_observation_kind(run, run.lease_fencing_token, 1, RunEventKind::Stopped)
            .await?;
        harness.core.collect_run_evidence(run.id).await?;
        let relative = PathBuf::from(run.id.to_string()).join(run.environment_epoch.to_string());
        assert!(
            !root
                .join("grants")
                .join(&relative)
                .join("claude-setup-token")
                .exists()
        );
        assert!(
            !root
                .join("runtime")
                .join(&relative)
                .join("claude-home/setup-token")
                .exists()
        );
        assert!(
            root.join("evidence")
                .join(relative)
                .join("stdout.jsonl")
                .exists()
        );
        let report: Value =
            sqlx::query_scalar("SELECT report FROM run_runtime_reports WHERE run_id=$1")
                .bind(run.id)
                .fetch_one(&harness.pool)
                .await?;
        assert_eq!(report["completed_turns"], 1);
        assert_eq!(report["usage"]["input_tokens"], 7);
    }
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM provider_credentials WHERE project_id=$1 ORDER BY secret_id",
    )
    .bind(project.as_uuid())
    .fetch_all(&harness.pool)
    .await?;
    assert_eq!(
        versions,
        [1, 1],
        "setup-token has no OAuth refresh/writeback"
    );
    assert!(
        source.path().exists(),
        "operator enrollment source is never removed"
    );
    Ok(())
}
