//! Canonical four-lane enrollment/registration with synthetic bytes and no Supervisor.
use anyhow::{Context, Result, ensure};
use forge_cli::profile_template::{ProfileLane, ProfileTemplateInput, build};
use forge_domain::{
    ProjectId,
    runtime::{ResourceLimits, RunBudget, SurfaceAccess},
};
use forge_protocol::wire::CommandName;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_testkit::m0::M0Harness;
use serde_json::{Value, json};
use std::{os::unix::fs::DirBuilderExt, path::Path};
use uuid::Uuid;

const ACCOUNT: &str = "synthetic-m2-account";
const API_KEY: &str = "sk-synthetic-m2-no-provider";
const CLAUDE: &str = "sk-ant-oat01-synthetic-m2-no-provider";

async fn register(
    harness: &M0Harness,
    root: &Path,
    project: ProjectId,
    lane: ProfileLane,
) -> Result<()> {
    let (kind, credential, account) = match lane {
        ProfileLane::CodexCli => ("codex_chatgpt", json!({"auth_mode":"chatgpt","last_refresh":"2026-09-06T00:00:00Z","tokens":{"account_id":ACCOUNT,"access_token":"synthetic-access","refresh_token":"synthetic-refresh","id_token":"synthetic-id"}}).to_string(), Some(ACCOUNT.into())),
        ProfileLane::ClaudeCli => ("claude_subscription", CLAUDE.into(), Some(ACCOUNT.into())),
        _ => ("api_key", API_KEY.into(), None),
    };
    let secret_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    let source = PrivateMaterialization::create(
        &root.join(secret_id.to_string()),
        &SecretBytes::new(credential.into_bytes()),
    )?;
    harness.execute(project, CommandName::EnrollCredential, json!({"secret_id":secret_id,"binding_id":binding_id,"kind":kind,"source_file":source.path()})).await?;
    let employee: Uuid = harness.execute(project, CommandName::CreateEmployee, json!({"name":format!("{lane:?}"),"role":"executor","stage_eligibility":{"mode":"any"}})).await?.resource.context("employee")?.id.parse()?;
    let template = build(ProfileTemplateInput {
        lane,
        project_id: project,
        employee_id: employee,
        binding_id,
        secret_id,
        account_id: account,
        model: "explicit-fixture-model".into(),
        image: format!("localhost/fixture@sha256:{}", "0".repeat(64)),
        system_prompt: "Synthetic registration; never start a provider.".into(),
        employee_prompt: "Synthetic registration only.".into(),
        limits: ResourceLimits::default(),
        budget: RunBudget::default(),
        live_input: true,
        access: SurfaceAccess::ReadWrite,
    })?;
    let payload = serde_json::to_value(&template)?;
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            payload.clone(),
        )
        .await?;
    let mut tx = harness.store.begin().await?;
    let stored = tx
        .runtime_binding(employee.into())
        .await?
        .context("binding")?;
    assert_eq!(stored, template.binding);
    tx.commit().await?;
    let rendered = serde_json::to_string(&stored)?;
    for secret in [
        API_KEY,
        CLAUDE,
        "synthetic-access",
        "synthetic-refresh",
        "synthetic-id",
    ] {
        ensure!(
            !rendered.contains(secret),
            "secret appeared in runtime profile"
        );
    }
    let mut changed = payload.clone();
    changed["binding"]["execution_profile"]["model"] = json!("another-model");
    assert!(
        harness
            .execute(project, CommandName::ConfigureEmployeeRuntime, changed)
            .await
            .is_err()
    );
    let mut unsupported = payload;
    unsupported["binding"]["execution_profile"]["id"] = json!(Uuid::now_v7());
    unsupported["binding"]["execution_profile"]["capability_profile"]["capabilities"]
        .as_array_mut()
        .context("capabilities")?
        .push(json!("session_resume"));
    assert!(
        harness
            .execute(project, CommandName::ConfigureEmployeeRuntime, unsupported)
            .await
            .is_err()
    );
    let stored: Value =
        sqlx::query_scalar("SELECT binding FROM employee_runtime_bindings WHERE employee_id=$1")
            .bind(employee)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(stored, serde_json::to_value(template.binding)?);
    let api = forge_testkit::m0::LocalHttpApi::start(harness.core.clone())?;
    let client = reqwest::Client::builder()
        .unix_socket(api.socket())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    let response = client
        .get(format!(
            "http://localhost/v1/projects/{project}/employees/{employee}/runtime-metadata"
        ))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let metadata: Value = serde_json::from_str(&response)?;
    assert_eq!(metadata["availability"], "available");
    assert_eq!(metadata["model"], "explicit-fixture-model");
    assert_eq!(metadata["employee_id"], json!(employee));
    for private in [
        "system_prompt",
        "employee_prompt",
        "secret_id",
        "account_id",
        "repository",
        "synthetic-access",
        "synthetic-refresh",
        "synthetic-id",
    ] {
        assert!(
            !response.contains(private),
            "runtime metadata leaked private field: {private}"
        );
    }
    let foreign = client
        .get(format!(
            "http://localhost/v1/projects/{}/employees/{employee}/runtime-metadata",
            ProjectId::new()
        ))
        .send()
        .await?;
    assert_eq!(foreign.status(), reqwest::StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic enrollment only, no model or Supervisor"]
async fn four_profile_templates_register_immutable_scoped_live_input_bindings() -> Result<()> {
    let root = std::env::temp_dir().join(format!("fm2p-{}", Uuid::now_v7()));
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let secrets = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let harness =
        M0Harness::start_configured(move |core| Ok(core.with_secret_store(secrets))).await?;
    let project = harness.create_project("Four offline M2 profiles").await?;
    for lane in [
        ProfileLane::CodexCli,
        ProfileLane::ClaudeCli,
        ProfileLane::OpenRouterApi,
        ProfileLane::OpenAiApi,
    ] {
        register(&harness, &root, project, lane).await?;
    }
    assert!(
        harness
            .store
            .list_runs_for_project(project)
            .await?
            .is_empty()
    );
    assert!(harness.store.list_tasks(project).await?.is_empty());
    Ok(())
}
