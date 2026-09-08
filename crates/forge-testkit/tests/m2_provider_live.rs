//! Explicitly paid opt-in. One selected lane, two Tasks of one Employee, same-Run input.
use anyhow::{Context, Result, ensure};
use forge_domain::{
    ContextSnapshot, LifecycleStatus, ProjectId, TaskId, communication::TaskMessageContext,
};
use forge_protocol::wire::CommandName;
use forge_provider_common::{MasterKeySource, PrivateMaterialization, SecretStore};
use forge_storage::RunProjection;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[path = "m2_provider_live/settings.rs"]
mod settings;
// Same independently reviewed bounded cleanup; no new broad container selector.
#[path = "m1_codex_live/cleanup.rs"]
mod cleanup;

fn container(run: &RunProjection) -> String {
    format!(
        "forge-run-{}-{}-{}",
        run.id, run.lease_fencing_token, run.environment_epoch
    )
}

fn marker(root: &Path, run: &RunProjection, name: &str) -> Result<PathBuf> {
    let surface = run.run_spec["surface_id"]
        .as_str()
        .context("Task surface")?;
    Ok(root
        .join("surfaces")
        .join(surface)
        .join("worktree")
        .join(name))
}

async fn await_marker(root: &Path, runs: &[RunProjection], name: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if runs
                .iter()
                .map(|run| marker(root, run, name).map(|path| path.is_file()))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .all(|exists| exists)
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("provider did not execute the bounded marker instruction")?
}

async fn send_followup(harness: &M0Harness, run: &RunProjection) -> Result<Uuid> {
    let snapshot: ContextSnapshot = serde_json::from_value(run.context_manifest.clone())?;
    let data = snapshot.data();
    let context = TaskMessageContext {
        task_id: data.task_id,
        pipeline_version_id: data.pipeline_version_id,
        stage_id: data.stage_id.clone(),
        stage_visit: data.stage_visit.context("pinned visit")?,
    };
    let thread: Uuid = harness
        .execute(
            run.project_id,
            CommandName::OpenEmployeeThread,
            json!({"employee_id":run.employee_id,"task_id":data.task_id}),
        )
        .await?
        .resource
        .context("thread")?
        .id
        .parse()?;
    let message = harness.execute(run.project_id,CommandName::SendEmployeeMessage,json!({
        "thread_id":thread,"expected_thread_revision":1,
        "target":{"kind":"exact_run","context":context,"run_id":run.id,"fencing_token":run.lease_fencing_token,"environment_epoch":run.environment_epoch},
        "kind":"instruction","requirement":"informational","body":"Execute exactly this shell command: `printf 'followup\\n' > forge-live-followup`. Then report ready. Do not change any other files or complete the Forge Task."
    })).await?.resource.context("follow-up")?.id.parse()?;
    Ok(message)
}

async fn exercise(
    harness: &M0Harness,
    project: ProjectId,
    tasks: &[TaskId; 2],
    root: &Path,
    runs: &mut Vec<RunProjection>,
) -> Result<()> {
    harness.start_project(project).await?;
    for task in tasks {
        runs.push(harness.wait_for_run_count(*task, 1).await?.remove(0));
    }
    ensure!(
        runs[0].employee_id == runs[1].employee_id,
        "expected the same Employee"
    );
    ensure!(
        runs[0].run_spec["surface_id"] != runs[1].run_spec["surface_id"],
        "shared Task surface"
    );
    await_marker(root, runs, "forge-live-ready").await?;
    let message = send_followup(harness, &runs[0]).await?;
    await_marker(root, &runs[..1], "forge-live-followup").await?;
    ensure!(
        !marker(root, &runs[1], "forge-live-followup")?.exists(),
        "targeted input reached another Task"
    );
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let accepted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_input_deliveries d JOIN runtime_input_observations o ON o.input_id=d.id WHERE d.run_id=$1 AND d.source_message_id=$2 AND o.outcome='runtime_accepted')").bind(runs[0].id).bind(message).fetch_one(&harness.pool).await?;
            if accepted {return Ok::<_,anyhow::Error>(());}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.context("native RuntimeAccepted observation missing")??;
    ensure!(
        harness.store.list_runs_for_project(project).await?.len() == 2,
        "native follow-up created a new Run"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "REAL paid/subscription usage; only explicit just test-m2-live, never synthetic integration"]
async fn live_m2_selected_provider_preserves_session_input_and_isolation() -> Result<()> {
    let settings = settings::Settings::environment()?;
    forge_testkit::m0::require_integration()?;
    let original = PrivateMaterialization::open(&settings.credential)?.read(1024 * 1024)?;
    let root = std::env::temp_dir().join(format!("fl2-{}", Uuid::now_v7()));
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    eprintln!("Live M2 private evidence root: {}", root.display());
    let secrets = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let proxy = settings.proxy.clone();
    let mut harness = M0Harness::start_configured(move |core| {
        let core = core
            .with_secret_store(secrets)
            .with_execution_root(shared)?;
        Ok(if let Some((endpoint, master)) = proxy {
            core.with_inference_proxy(&endpoint, &master)?
        } else {
            core
        })
    })
    .await?;
    let project = harness
        .create_project("Explicit selected M2 live lane")
        .await?;
    let secret = Uuid::now_v7();
    let credential = Uuid::now_v7();
    harness.execute(project,CommandName::EnrollCredential,json!({"secret_id":secret,"binding_id":credential,"kind":settings.credential_kind(),"source_file":settings.credential})).await?;
    let employee: Uuid = harness.execute(project,CommandName::CreateEmployee,json!({"name":"M2 explicit live Employee","role":"executor","stage_eligibility":{"mode":"any"}})).await?.resource.context("employee")?.id.parse()?;
    harness.execute(project,CommandName::AmendEmployee,json!({"employee_id":employee,"expected_employee_revision":1,"patch":{"max_concurrent_runs":2}})).await?;
    let template = settings.binding(project, employee, secret, credential)?;
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            serde_json::to_value(template)?,
        )
        .await?;
    harness.attach_runtime_supervisor(root.clone()).await?;
    let host = cleanup::supervisor_host(&root)?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let tasks = [
        harness
            .create_task(project, pipeline, "Live input Task A")
            .await?,
        harness
            .create_task(project, pipeline, "Isolated Task B")
            .await?,
    ];
    for task in tasks {
        harness.approve_task(project, task).await?;
    }
    let mut runs = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(210),
        exercise(&harness, project, &tasks, &root, &mut runs),
    )
    .await;
    // Always stop, even after model, network, receipt or assertion failure.
    let reports =
        cleanup::stop_and_collect(&mut harness, project, &tasks, runs, &root, &host).await?;
    result.context("entire live exercise exceeded its deadline")??;
    ensure!(reports.len() == 2, "two Run evidence reports required");
    for task in tasks {
        ensure!(
            harness.required_task(task).await?.task.lifecycle() == LifecycleStatus::Waiting,
            "process exit closed Task"
        );
    }
    let after = PrivateMaterialization::open(&settings.credential)?.read(1024 * 1024)?;
    ensure!(
        original.expose() == after.expose(),
        "explicit credential source changed"
    );
    let rendered = serde_json::to_string(&reports)?;
    if settings.credential_kind() == "codex_chatgpt" {
        let auth: Value = serde_json::from_slice(original.expose())?;
        for field in ["access_token", "refresh_token", "id_token"] {
            if let Some(value) = auth["tokens"][field].as_str() {
                ensure!(
                    !rendered.contains(value),
                    "credential leaked into diagnostics"
                );
            }
        }
    } else {
        let token = std::str::from_utf8(original.expose())?.trim();
        ensure!(
            !token.is_empty() && !rendered.contains(token),
            "credential leaked into diagnostics"
        );
    }
    Ok(())
}
