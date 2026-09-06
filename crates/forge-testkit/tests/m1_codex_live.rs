//! Opt-in REAL subscription smoke. Never run as part of synthetic integration.
//! The explicit auth snapshot is read only after all four settings are validated.

use anyhow::{Context, Result, ensure};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, LifecycleStatus, ProjectId,
    TaskId,
    runtime::{ResourceLimits, RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::wire::CommandName;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore};
use forge_storage::RunProjection;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[path = "m1_codex_live/cleanup.rs"]
mod cleanup;

struct Settings {
    auth: PathBuf,
    model: String,
    image: String,
}
impl Settings {
    fn parse(
        enabled: Option<&str>,
        auth: Option<&str>,
        model: Option<&str>,
        image: Option<&str>,
    ) -> Result<Self> {
        ensure!(
            enabled == Some("1"),
            "real subscription execution requires FORGE_CODEX_LIVE=1"
        );
        let auth = PathBuf::from(auth.context("set explicit FORGE_CODEX_AUTH_FILE")?);
        ensure!(
            auth.is_absolute(),
            "FORGE_CODEX_AUTH_FILE must be absolute; no default auth discovery"
        );
        let model = model.context("set explicit FORGE_CODEX_MODEL")?;
        ensure!(
            !model.trim().is_empty()
                && model.trim() == model
                && !model.chars().any(char::is_control),
            "invalid explicit model"
        );
        let image = image.context("set explicit FORGE_CODEX_RUNTIME_IMAGE")?;
        ensure!(
            !image.starts_with('-')
                && !image.chars().any(char::is_whitespace)
                && image
                    .rsplit_once("@sha256:")
                    .is_some_and(|(name, digest)| !name.is_empty()
                        && digest.len() == 64
                        && digest.bytes().all(|b| b.is_ascii_hexdigit())),
            "FORGE_CODEX_RUNTIME_IMAGE must be digest pinned"
        );
        Ok(Self {
            auth,
            model: model.into(),
            image: image.into(),
        })
    }
    fn environment() -> Result<Self> {
        Self::parse(
            std::env::var("FORGE_CODEX_LIVE").ok().as_deref(),
            std::env::var("FORGE_CODEX_AUTH_FILE").ok().as_deref(),
            std::env::var("FORGE_CODEX_MODEL").ok().as_deref(),
            std::env::var("FORGE_CODEX_RUNTIME_IMAGE").ok().as_deref(),
        )
    }
}

async fn enroll(harness: &M0Harness, project: ProjectId, settings: &Settings) -> Result<()> {
    let secret_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    harness.execute(project, CommandName::EnrollCredential, json!({
        "secret_id":secret_id,"binding_id":binding_id,"kind":"codex_chatgpt","source_file":settings.auth
    })).await?;
    // Read only the enrolled identity, not a second raw-token representation.
    let account: String = sqlx::query_scalar("SELECT sealed_record #>> '{snapshot,account_id}' FROM provider_credentials WHERE secret_id=$1")
        .bind(secret_id).fetch_one(&harness.pool).await?;
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let profile = ExecutionProfileInput {
        id: Uuid::now_v7(),
        revision: 1,
        project_id: project,
        adapter_id: "codex_cli".into(),
        adapter_version: "0.153.2".into(),
        provider_id: "openai".into(),
        model: settings.model.clone(),
        credential_binding: CredentialBinding {
            id: binding_id,
            project_id: project,
            secret_id,
            account_id: Some(account),
            allowed_delivery_modes: BTreeSet::from([mode]),
        },
        credential_delivery: mode,
        capability_profile: forge_provider_codex::CodexAdapter::capabilities(),
    }
    .try_into()?;
    let binding = RuntimeBinding {
        budget: Default::default(), execution_profile: profile, image: settings.image.clone(),
        surface: SurfaceSpec::FilesystemSandbox, access: SurfaceAccess::ReadWrite,
        limits: ResourceLimits { wall_seconds: 180, stop_grace_seconds: 5, ..Default::default() },
        system_prompt: "This is a bounded live smoke test. Use the shell only inside your assigned working directory. Never inspect credentials or other paths. Do not submit artifacts or a task outcome.".into(),
        employee_prompt: "Run exactly this one shell command in your working directory: `printf 'ready\\n' > forge-live-ready; sleep 120`. The sleep is intentional: the operator will stop this Run. Do not repeat or run other commands.".into(),
    };
    for name in ["Live Codex A", "Live Codex B"] {
        harness.create_employee(project, name).await?;
    }
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

fn scoped(root: &Path, section: &str, run: &RunProjection) -> PathBuf {
    root.join(section)
        .join(run.id.to_string())
        .join(run.environment_epoch.to_string())
}
fn container(run: &RunProjection) -> String {
    format!(
        "forge-run-{}-{}-{}",
        run.id, run.lease_fencing_token, run.environment_epoch
    )
}

async fn exercise(
    harness: &M0Harness,
    project: ProjectId,
    tasks: &[TaskId; 2],
    root: &Path,
    auth: &SecretBytes,
    runs: &mut Vec<RunProjection>,
) -> Result<()> {
    harness.start_project(project).await?;
    runs.push(harness.wait_for_run_count(tasks[0], 1).await?.remove(0));
    runs.push(harness.wait_for_run_count(tasks[1], 1).await?.remove(0));
    let (a, b) = (&runs[0], &runs[1]);
    eprintln!(
        "Live containers (retained after stop): {}, {}",
        container(a),
        container(b)
    );
    ensure!(
        a.run_spec["surface_id"] != b.run_spec["surface_id"],
        "Runs shared a work surface"
    );
    let homes = [a, b].map(|run| scoped(root, "runtime", run).join("codex-home"));
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let ready = [a, b].into_iter().all(|run| {
                run.run_spec["surface_id"].as_str().is_some_and(|surface| {
                    root.join("surfaces")
                        .join(surface)
                        .join("worktree/forge-live-ready")
                        .is_file()
                })
            });
            if ready {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("both real Codex sessions must execute their marker command")??;
    let identities = homes
        .iter()
        .map(fs::metadata)
        .collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        (identities[0].dev(), identities[0].ino()) != (identities[1].dev(), identities[1].ino()),
        "Codex homes share an inode"
    );
    for run in [a, b] {
        let initial = PrivateMaterialization::open(&scoped(root, "grants", run).join("auth.json"))?
            .read(1024 * 1024)?;
        ensure!(
            initial.expose() == auth.expose(),
            "Run did not receive the explicit shared snapshot"
        );
        let state = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("podman")
                .args(["inspect", "--format", "{{.State.Running}}", &container(run)])
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        ensure!(
            state.status.success() && state.stdout == b"true\n",
            "two provider Runs did not overlap before controlled stop"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "REAL subscription usage; only explicit just test-codex-live, never synthetic integration"]
async fn live_codex_subscription_two_parallel_runs_isolated_stop_and_evidence() -> Result<()> {
    let settings = Settings::environment()?;
    forge_testkit::m0::require_integration()?;
    // No access to this file happens before explicit opt-in and complete settings.
    let auth = PrivateMaterialization::open(&settings.auth)?.read(1024 * 1024)?;
    let root = std::env::temp_dir().join(format!("fc-{}", Uuid::now_v7()));
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    eprintln!("Live Codex private evidence root: {}", root.display());
    let store = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let mut harness = M0Harness::start_configured(move |core| {
        Ok(core.with_secret_store(store).with_execution_root(shared)?)
    })
    .await?;
    let project = harness.create_project("Explicit live Codex smoke").await?;
    enroll(&harness, project, &settings).await?;
    harness.attach_runtime_supervisor(root.clone()).await?;
    let host_id = cleanup::supervisor_host(&root)?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let tasks = [
        harness
            .create_task(project, pipeline, "Live Codex marker A")
            .await?,
        harness
            .create_task(project, pipeline, "Live Codex marker B")
            .await?,
    ];
    for task in tasks {
        harness.approve_task(project, task).await?;
    }
    // Always execute named stop even when either provider fails or times out.
    let mut runs = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(150),
        exercise(&harness, project, &tasks, &root, &auth, &mut runs),
    )
    .await;
    let cleanup =
        cleanup::stop_and_collect(&mut harness, project, &tasks, runs, &root, &host_id).await;
    let reports = cleanup?;
    result.context("entire live exercise exceeded its deadline")??;
    ensure!(
        reports.len() == 2,
        "expected evidence for exactly two live Runs"
    );
    for task in tasks {
        ensure!(
            harness.required_task(task).await?.task.lifecycle() == LifecycleStatus::Waiting,
            "live Run exit closed Task"
        );
    }
    let source = PrivateMaterialization::open(&settings.auth)?.read(1024 * 1024)?;
    ensure!(
        source.expose() == auth.expose(),
        "explicit source snapshot changed"
    );
    // Compare without formatting assertions: failures must never print tokens.
    let document: Value =
        serde_json::from_slice(auth.expose()).context("invalid auth snapshot JSON")?;
    let evidence = serde_json::to_string(&reports)?;
    for field in ["access_token", "refresh_token", "id_token"] {
        if let Some(token) = document["tokens"][field].as_str() {
            ensure!(
                !evidence.contains(token),
                "credential bytes leaked into diagnostics"
            );
        }
    }
    Ok(())
}

#[test]
fn live_settings_never_default_to_host_auth_or_model() {
    assert!(Settings::parse(None, None, None, None).is_err());
    assert!(Settings::parse(Some("1"), None, None, None).is_err());
    assert!(
        Settings::parse(
            Some("1"),
            Some("relative/auth.json"),
            Some("chosen"),
            Some("image:m1")
        )
        .is_err()
    );
    assert!(Settings::parse(Some("1"), Some("/explicit/auth.json"), None, None).is_err());
}
