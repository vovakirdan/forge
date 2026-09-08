use anyhow::{Context, Result};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, EmployeeId, ExecutionProfileInput, ProjectId,
    runtime::{RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::wire::CommandName;
use forge_provider_claude::ClaudeAdapter;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_storage::RunProjection;
use forge_testkit::m0::M0Harness;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{collections::BTreeSet, os::unix::fs::DirBuilderExt, path::PathBuf, time::Duration};
use uuid::Uuid;

pub(super) async fn fixture() -> Result<(M0Harness, ProjectId, EmployeeId, PathBuf)> {
    fixture_mode(false).await
}
pub(super) async fn fixture_mode(
    live: bool,
) -> Result<(M0Harness, ProjectId, EmployeeId, PathBuf)> {
    let root = std::env::temp_dir().join(format!("fcom-{}", Uuid::now_v7()));
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let secrets = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let harness = M0Harness::start_configured(move |core| {
        Ok(core
            .with_secret_store(secrets)
            .with_execution_root(shared)?)
    })
    .await?;
    let project = harness
        .create_project("Taskless communication contract")
        .await?;
    harness.create_employee(project, "Bob").await?;
    let employee = harness
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let secret_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    let binding = RuntimeBinding {
        budget: Default::default(),
        limits: Default::default(),
        image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
        surface: SurfaceSpec::FilesystemSandbox,
        access: SurfaceAccess::ReadWrite,
        system_prompt: "Synthetic contract; no provider calls.".into(),
        employee_prompt: "Reply using scoped Inbox tools.".into(),
        execution_profile: ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: project,
            adapter_id: "claude_code_cli".into(),
            adapter_version: "2.1.263".into(),
            provider_id: "anthropic".into(),
            model: "synthetic-no-inference".into(),
            credential_delivery: mode,
            capability_profile: if live {
                ClaudeAdapter::live_capabilities()
            } else {
                ClaudeAdapter::capabilities()
            },
            credential_binding: CredentialBinding {
                id: binding_id,
                project_id: project,
                secret_id,
                account_id: Some("synthetic-account".into()),
                allowed_delivery_modes: BTreeSet::from([mode]),
            },
        }
        .try_into()?,
    };
    let credential = PrivateMaterialization::create(
        &root.join("synthetic-token"),
        &SecretBytes::new(b"sk-ant-oat01-synthetic-communication-no-real-auth".to_vec()),
    )?;
    harness.execute(project,CommandName::EnrollCredential,json!({"secret_id":secret_id,"binding_id":binding_id,"kind":"claude_subscription","source_file":credential.path()})).await?;
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":employee,"binding":binding}),
        )
        .await?;
    Ok((harness, project, employee, root))
}

pub(super) async fn send_question(
    harness: &M0Harness,
    project: ProjectId,
    employee: EmployeeId,
    task: forge_domain::TaskId,
) -> Result<Uuid> {
    let thread: Uuid = harness
        .execute(
            project,
            CommandName::OpenEmployeeThread,
            json!({"employee_id":employee,"task_id":task}),
        )
        .await?
        .resource
        .context("thread")?
        .id
        .parse()?;
    Ok(harness.execute(project,CommandName::SendEmployeeMessage,json!({"thread_id":thread,"expected_thread_revision":1,
        "target":{"kind":"inbox"},"kind":"question","requirement":"answered","body":"Explain the current project plan; do not modify the Task."})).await?
        .resource.context("source message")?.id.parse()?)
}

pub(super) fn client(root: &std::path::Path, run: &RunProjection) -> Result<Client> {
    Ok(Client::builder()
        .unix_socket(
            root.join("gateways")
                .join(run.id.to_string())
                .join("gateway.sock"),
        )
        .timeout(Duration::from_secs(8))
        .build()?)
}
pub(super) async fn call(
    client: &Client,
    tool: &str,
    args: Value,
    id: Uuid,
) -> Result<(StatusCode, Value)> {
    let response = client
        .post("http://localhost/tools")
        .json(&json!({"tool":tool,"arguments":args,"message_id":id}))
        .send()
        .await?;
    Ok((response.status(), response.json().await?))
}
