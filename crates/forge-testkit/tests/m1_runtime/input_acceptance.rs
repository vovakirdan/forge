//! Real Core/PG/gRPC/Supervisor/Podman; synthetic CLI and in-memory object storage.
//! Real MinIO's byte-exact boundary is covered separately by evidence_minio.

use super::*;
use forge_domain::{ArtifactId, TaskId, file_snapshot::FileSnapshotManifest};
use forge_storage::{RunObservedState, RunProjection, evidence::ObjectEvidenceStore};
use serde_json::Value;
use std::{path::Path, process::Output, sync::Arc};
use tokio::{process::Command, time::sleep};

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS and rootless Podman fixture image; no provider"]
async fn core_snapshots_reach_real_readonly_mounts_and_named_stop_enables_capture() -> Result<()> {
    let root = directory()?;
    let secrets = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let object_backend = Arc::new(object_store::memory::InMemory::new());
    let objects = ObjectEvidenceStore::from_store(object_backend.clone());
    let mut harness = M0Harness::start_configured(|core| {
        Ok(core
            .with_secret_store(secrets)
            .with_execution_root(root.clone())?
            .with_test_object_store(ObjectEvidenceStore::from_store(object_backend))?)
    })
    .await?;
    let project = harness
        .create_project("Physical file snapshot acceptance")
        .await?;
    let image = podman(&[
        "image",
        "inspect",
        "localhost/forge-runner-fixture:m1",
        "--format",
        "{{.Digest}}",
    ])
    .await?;
    anyhow::ensure!(image.status.success(), "build Containerfile.fixture first");
    enroll(
        &harness,
        &root,
        project,
        &format!(
            "localhost/forge-runner-fixture@{}",
            std::str::from_utf8(&image.stdout)?.trim()
        ),
    )
    .await?;
    for employee in harness.store.list_employees(project).await? {
        let id = employee.employee.id();
        let mut tx = harness.store.begin().await?;
        let binding = tx
            .runtime_binding(id)
            .await?
            .context("fixture runtime binding")?;
        tx.commit().await?;
        let mut binding = serde_json::to_value(binding)?;
        binding["employee_prompt"] = json!("FORGE_ACCEPTANCE_HOLD_FOR_STOP");
        binding["limits"]["wall_seconds"] = json!(90);
        binding["limits"]["stop_grace_seconds"] = json!(1);
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":id,"binding":binding}),
            )
            .await?;
    }
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let source_task = harness
        .create_task(project, pipeline, "Snapshot source")
        .await?;
    let receiver = harness
        .create_task(project, pipeline, "Consume immutable input files")
        .await?;
    harness.attach_runtime_supervisor(root.clone()).await?;
    let outcome = tokio::time::timeout(
        Duration::from_secs(90),
        exercise(&harness, &root, project, source_task, receiver, &objects),
    )
    .await
    .context("bounded physical snapshot exercise")
    .and_then(|result| result);
    // Run this even when an assertion returns Err; retain stopped surfaces as evidence.
    let cleanup = cleanup(&mut harness, project, receiver).await;
    harness.shutdown().await;
    outcome?;
    cleanup
}

async fn exercise(
    harness: &M0Harness,
    root: &Path,
    project: ProjectId,
    source_task: TaskId,
    receiver: TaskId,
    objects: &ObjectEvidenceStore,
) -> Result<()> {
    let source = tempfile::tempdir()?;
    for (path, bytes) in [
        ("text.txt", b"Frozen text input\n".as_slice()),
        ("binary.bin", &[0, 255, 1, 128]),
        ("empty.txt", &[]),
    ] {
        fs::write(source.path().join(path), bytes)?;
    }
    harness
        .execute(
            project,
            CommandName::ImportTaskFileSnapshot,
            json!({"task_id":source_task,"expected_task_revision":1,"title":"Frozen input",
        "root":source.path(),"paths":["text.txt","binary.bin","empty.txt"]}),
        )
        .await?;
    anyhow::ensure!(
        harness.core.file_snapshot_tick().await? == 1,
        "import must finish"
    );
    let views = harness
        .core
        .read_file_snapshots(project, source_task, None, 50)
        .await?;
    anyhow::ensure!(
        views.len() == 1 && views[0]["state"] == "sealed",
        "snapshot must seal"
    );
    let artifact: ArtifactId = serde_json::from_value(views[0]["artifact_id"].clone())?;
    harness
        .execute(
            project,
            CommandName::AttachTaskFileInput,
            json!({"task_id":receiver,"expected_task_revision":1,"artifact_id":artifact}),
        )
        .await?;
    fs::write(source.path().join("text.txt"), b"Changed original")?;
    fs::write(source.path().join("binary.bin"), [8, 9])?;
    fs::write(source.path().join("empty.txt"), b"No longer empty")?;
    harness.approve_task(project, receiver).await?;
    harness.start_project(project).await?;
    let run = harness.wait_for_run_count(receiver, 1).await?.remove(0);
    let name = container(&run);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let current = harness.store.load_run(run.id).await?.context("Run")?;
            if current.observed_state == RunObservedState::Running {
                break Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("fixture must really start")??;
    let inputs = podman(&["exec", &name, "cat", "/run/forge-inputs/inputs.json"]).await?;
    anyhow::ensure!(inputs.status.success(), "read physical input manifest");
    anyhow::ensure!(
        serde_json::from_slice::<Value>(&inputs.stdout)? == run.run_spec["file_inputs"],
        "physical input identities must match Core's frozen RunSpec"
    );
    let input = format!("/run/forge-inputs/{artifact}/files");
    for (path, expected) in [
        ("text.txt", b"Frozen text input\n".as_slice()),
        ("binary.bin", &[0, 255, 1, 128]),
        ("empty.txt", &[]),
    ] {
        let output = podman(&["exec", &name, "cat", &format!("{input}/{path}")]).await?;
        anyhow::ensure!(
            output.status.success() && output.stdout == expected,
            "physical frozen bytes differ for {path}"
        );
    }
    let write = podman(&[
        "exec",
        &name,
        "sh",
        "-c",
        "printf changed > \"$1/text.txt\"",
        "probe",
        &input,
    ])
    .await?;
    anyhow::ensure!(
        !write.status.success()
            && String::from_utf8_lossy(&write.stderr).contains("Read-only file system"),
        "snapshot mutation must fail specifically at a read-only mount"
    );
    let probe = podman(&["exec", &name, "sh", "-c",
        "test ! -e /workspace/worktree/text.txt && test ! -e /workspace/worktree/binary.bin && test ! -e /workspace/worktree/empty.txt && cp \"$1/binary.bin\" /workspace/worktree/copied.bin && printf 'Runtime output\n' > /workspace/worktree/output.txt",
        "probe", &input]).await?;
    anyhow::ensure!(
        probe.status.success(),
        "no overlay; explicit copy and output must be writable"
    );
    let worktree = root
        .join("surfaces")
        .join(run.run_spec["surface_id"].as_str().context("surface id")?)
        .join("worktree");
    anyhow::ensure!(
        fs::read(worktree.join("copied.bin"))? == [0, 255, 1, 128],
        "explicit copy bytes"
    );
    let active = harness.required_task(receiver).await?.task;
    anyhow::ensure!(
        harness
            .execute(
                project,
                CommandName::CaptureTaskFileSnapshot,
                json!({"task_id":receiver,"expected_task_revision":active.revision().get(),
        "title":"Must not stop a writer","paths":["output.txt"]})
            )
            .await
            .is_err(),
        "capture must refuse a physically running Task"
    );
    let still_running = podman(&["inspect", "--format", "{{.State.Running}}", &name]).await?;
    anyhow::ensure!(
        still_running.status.success() && still_running.stdout == b"true\n",
        "capture must not stop runtime"
    );
    harness.execute(project, CommandName::PauseTask,
        json!({"task_id":receiver,"expected_task_revision":active.revision().get(),"mode":"graceful"})).await?;
    wait_quiescent(harness, &run).await?;
    let stopped = podman(&["inspect", "--format", "{{.State.Running}}", &name]).await?;
    anyhow::ensure!(
        stopped.status.success() && stopped.stdout == b"false\n",
        "named pause must physically stop"
    );
    let paused = harness.required_task(receiver).await?.task;
    anyhow::ensure!(
        paused.lifecycle() == LifecycleStatus::Waiting,
        "pause lifecycle"
    );
    harness
        .execute(
            project,
            CommandName::CaptureTaskFileSnapshot,
            json!({"task_id":receiver,"expected_task_revision":paused.revision().get(),
        "title":"Physically stopped output","paths":["output.txt","copied.bin"]}),
        )
        .await?;
    let manifest = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            harness.core.file_snapshot_tick().await?;
            let views = harness
                .core
                .read_file_snapshots(project, receiver, None, 50)
                .await?;
            if let Some(view) = views.first() {
                anyhow::ensure!(view["state"] != "failed", "physical capture failed: {view}");
                if view["state"] == "sealed" {
                    break Ok::<_, anyhow::Error>(serde_json::from_value::<FileSnapshotManifest>(
                        view["manifest"].clone(),
                    )?);
                }
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("physical Supervisor capture must seal")??;
    for (path, expected) in [
        ("output.txt", b"Runtime output\n".as_slice()),
        ("copied.bin", &[0, 255, 1, 128]),
    ] {
        let file = manifest
            .files
            .iter()
            .find(|file| file.path == path)
            .context("captured file")?;
        anyhow::ensure!(
            objects.read_snapshot_file(file).await? == expected,
            "stored captured bytes differ"
        );
    }
    anyhow::ensure!(
        harness.required_task(receiver).await?.task.lifecycle() == LifecycleStatus::Waiting,
        "capturing an Artifact is not a successful Task outcome"
    );
    Ok(())
}

fn container(run: &RunProjection) -> String {
    format!(
        "forge-run-{}-{}-{}",
        run.id, run.lease_fencing_token, run.environment_epoch
    )
}

async fn podman(args: &[&str]) -> Result<Output> {
    tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("podman")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("bounded Podman command")?
    .context("execute Podman")
}

async fn wait_quiescent(harness: &M0Harness, run: &RunProjection) -> Result<()> {
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
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("positive physical quiescence")?
}

async fn cleanup(harness: &mut M0Harness, project: ProjectId, task: TaskId) -> Result<()> {
    let stop = harness.stop_project(project).await;
    let runs = harness.store.list_runs(task).await;
    let mut failure = stop.err();
    if let Ok(runs) = &runs {
        for run in runs {
            if let Err(error) = wait_quiescent(harness, run).await {
                failure = Some(error);
                let _ = podman(&["kill", "--signal", "KILL", &container(run)]).await;
            }
        }
    }
    let shutdown = harness.stop_runtime_supervisor_for_cleanup().await;
    runs?;
    shutdown?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(())
}
