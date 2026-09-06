//! Actual Core→sandboxed OpenCode→Gateway→LiteLLM→synthetic upstream.
//! No host provider auth, login, public provider endpoint, or paid inference.

use anyhow::{Context, Result, ensure};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, LifecycleStatus, ProjectId,
    runtime::{ResourceLimits, RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::wire::CommandName;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_provider_opencode::OpenCodeAdapter;
use forge_storage::RunProjection;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use uuid::Uuid;

const PROXY: &str = "http://127.0.0.1:4007";
const MASTER: &str = "sk-forge-contract-master-not-a-real-key";
const UPSTREAM: &str = "synthetic-upstream-api-lane-not-a-real-key";

async fn enroll(harness: &M0Harness, root: &Path, project: ProjectId, image: &str) -> Result<()> {
    let secret_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    let source = PrivateMaterialization::create(
        &root.join("input-api-key"),
        &SecretBytes::new(UPSTREAM.as_bytes().to_vec()),
    )?;
    harness
        .execute(
            project,
            CommandName::EnrollCredential,
            json!({
                "secret_id":secret_id,"binding_id":binding_id,"kind":"api_key",
                "source_file":source.path()
            }),
        )
        .await?;
    harness
        .create_employee(project, "API lane employee")
        .await?;
    let mode = CredentialDeliveryMode::ProxyOnly;
    let profile = ExecutionProfileInput {
        id: Uuid::now_v7(),
        revision: 1,
        project_id: project,
        adapter_id: "opencode_runtime".into(),
        adapter_version: "1.18.29".into(),
        provider_id: "openai".into(),
        model: "gpt-4o-mini".into(),
        credential_binding: CredentialBinding {
            id: binding_id,
            project_id: project,
            secret_id,
            account_id: None,
            allowed_delivery_modes: BTreeSet::from([mode]),
        },
        credential_delivery: mode,
        capability_profile: OpenCodeAdapter::capabilities(),
    }
    .try_into()?;
    let binding = RuntimeBinding {
        budget: Default::default(),
        execution_profile: profile,
        image: image.into(),
        surface: SurfaceSpec::FilesystemSandbox,
        access: SurfaceAccess::ReadWrite,
        limits: ResourceLimits::default(),
        system_prompt: "Synthetic contract test; reply with fixture response.".into(),
        employee_prompt: "Use only the supplied Forge Gateway and inference endpoint.".into(),
    };
    let employee = harness.store.list_employees(project).await?.remove(0);
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":employee.employee.id(),"binding":binding}),
        )
        .await?;
    Ok(())
}

async fn grant_key(root: &Path, run: &RunProjection) -> Result<SecretBytes> {
    let grant = root
        .join("grants")
        .join(run.id.to_string())
        .join(run.environment_epoch.to_string());
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if grant.join("invocation.json").try_exists()? {
                let key = PrivateMaterialization::open(&grant.join("api-key"))?.read(16 * 1024)?;
                ensure!(
                    key.expose() != UPSTREAM.as_bytes(),
                    "upstream credential reached Run grant"
                );
                ensure!(
                    key.expose() != MASTER.as_bytes(),
                    "master credential reached Run grant"
                );
                ensure!(
                    !grant.join("auth.json").exists(),
                    "API lane received subscription auth"
                );
                for entry in fs::read_dir(&grant)? {
                    let bytes = fs::read(entry?.path())?;
                    let text = String::from_utf8_lossy(&bytes);
                    ensure!(
                        !text.contains(UPSTREAM) && !text.contains(MASTER),
                        "grant leaked admin/upstream auth"
                    );
                }
                return Ok::<_, anyhow::Error>(key);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("wait for private virtual-key materialization")?
}

async fn assert_key_scope(
    harness: &M0Harness,
    run: &RunProjection,
    key: &SecretBytes,
) -> Result<String> {
    let spec: Value = sqlx::query_scalar("SELECT spec FROM run_proxy_keys WHERE run_id=$1")
        .bind(run.id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(spec["run_id"], run.id.to_string());
    assert_eq!(spec["environment_epoch"], run.environment_epoch);
    assert_eq!(spec["fencing_token"], run.lease_fencing_token);
    let models = spec["models"].as_array().context("restricted models")?;
    assert_eq!(models.len(), 1);
    let alias = models[0]
        .as_str()
        .context("immutable model alias")?
        .to_owned();
    ensure!(alias.starts_with("forge-route-"));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?;
    let wrong_model = client
        .post(format!("{PROXY}/v1/chat/completions"))
        .bearer_auth(std::str::from_utf8(key.expose())?)
        .json(&json!({"model":"forge-contract-stub","messages":[{"role":"user","content":"deny"}]}))
        .send()
        .await?;
    ensure!(
        !wrong_model.status().is_success(),
        "per-Run key allowed another model"
    );
    let admin = client
        .get(format!("{PROXY}/model/info"))
        .bearer_auth(std::str::from_utf8(key.expose())?)
        .send()
        .await?;
    ensure!(
        !admin.status().is_success(),
        "per-Run key reached proxy administration"
    );
    Ok(alias)
}

async fn wait_quiescent(harness: &M0Harness, run: &RunProjection) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let released: bool = sqlx::query_scalar(
                "SELECT released_at IS NOT NULL FROM run_environment_reservations WHERE run_id=$1",
            )
            .bind(run.id)
            .fetch_one(&harness.pool)
            .await?;
            if released {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("wait for positive physical quiescence")?
}

async fn assert_revoked(
    harness: &M0Harness,
    run: &RunProjection,
    key: &SecretBytes,
    alias: &str,
) -> Result<()> {
    // Quiescence commits before the external broker acknowledges deletion.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let revoked: bool = sqlx::query_scalar(
                "SELECT revoked_at IS NOT NULL AND NOT issuance_pending FROM run_proxy_keys WHERE run_id=$1"
            ).bind(run.id).fetch_one(&harness.pool).await?;
            if revoked { return Ok::<_, anyhow::Error>(()); }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.context("wait for durable proxy revocation acknowledgement")??;
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?
        .post(format!("{PROXY}/v1/chat/completions"))
        .bearer_auth(std::str::from_utf8(key.expose())?)
        .json(&json!({"model":alias,"messages":[{"role":"user","content":"must not execute"}]}))
        .send()
        .await?;
    ensure!(
        !response.status().is_success(),
        "physically stopped Run key remained usable"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL/NATS, pinned runtime image, internal-only LiteLLM fixture; run serially"]
async fn actual_api_run_is_scoped_accounted_and_revoked_without_task_success() -> Result<()> {
    ensure!(std::env::var("FORGE_LITELLM_CONTRACT").as_deref() == Ok("1"));
    let image = std::env::var("FORGE_OPENCODE_FIXTURE_IMAGE")
        .context("provide the built runtime digest")?;
    ensure!(image.contains("@sha256:"), "runtime must be digest pinned");
    let health = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if health
                .get(format!("{PROXY}/health/liveliness"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .context("wait for dedicated LiteLLM fixture before creating Run authority")?;
    // The nested per-Run Unix socket must remain below Linux sun_path's bound.
    let root = std::env::temp_dir().join(format!("fa-{}", Uuid::now_v7()));
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let master = PrivateMaterialization::create(
        &root.join("proxy-master"),
        &SecretBytes::new(MASTER.as_bytes().to_vec()),
    )?;
    let store = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let mut harness = M0Harness::start_configured(move |core| {
        Ok(core
            .with_secret_store(store)
            .with_execution_root(shared)?
            .with_inference_proxy(PROXY, master.path())?)
    })
    .await?;
    let project = harness.create_project("M1 actual API transport").await?;
    enroll(&harness, &root, project, &image).await?;
    harness.attach_runtime_supervisor(root.clone()).await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(
            project,
            pipeline,
            "FORGE_FIXTURE_DELAY complete synthetic reply",
        )
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let key = grant_key(&root, &run).await?;
    let alias = assert_key_scope(&harness, &run, &key).await?;
    let canonical = serde_json::to_string(&run.run_spec)?;
    ensure!(!canonical.contains(UPSTREAM) && !canonical.contains(MASTER));
    ensure!(!canonical.contains(std::str::from_utf8(key.expose())?));
    let gateway = root
        .join("gateways")
        .join(run.id.to_string())
        .join("gateway.sock");
    let gateway_client = reqwest::Client::builder()
        .unix_socket(gateway)
        .timeout(Duration::from_secs(10))
        .build()?;
    let denied = gateway_client
        .post("http://localhost/v1/chat/completions")
        .json(&json!({"model":"wrong-model","messages":[]}))
        .send()
        .await?;
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
    wait_quiescent(&harness, &run).await?;
    assert_revoked(&harness, &run, &key, &alias).await?;
    assert_eq!(
        harness.required_task(task).await?.task.lifecycle(),
        LifecycleStatus::Waiting
    );
    harness.core.collect_run_evidence(run.id).await?;
    let diagnostics = harness.store.run_diagnostics(run.id).await?;
    assert_eq!(
        diagnostics["runtime_report"]["provider_exit"]["exit_code"], 0,
        "actual OpenCode must complete successfully: {diagnostics}"
    );
    assert_eq!(diagnostics["runtime_report"]["completed_turns"], 1);
    assert_eq!(diagnostics["runtime_report"]["usage"]["input_tokens"], 1000);
    assert_eq!(
        diagnostics["runtime_report"]["usage"]["output_tokens"],
        1000
    );
    assert_eq!(diagnostics["handoff"]["outcome"]["kind"], "interrupted");
    ensure!(
        diagnostics["proxy_usage"].is_object(),
        "proxy spend observation missing"
    );
    let rendered = serde_json::to_string(&diagnostics)?;
    ensure!(!rendered.contains(UPSTREAM) && !rendered.contains(MASTER));
    ensure!(!rendered.contains(std::str::from_utf8(key.expose())?));
    // Real work surfaces and redacted diagnostic evidence are intentionally retained.
    Ok(())
}
