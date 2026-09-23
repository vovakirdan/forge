//! Canonical PostgreSQL commands with byte-exact isolated object storage; no inference.

use anyhow::{Context, Result};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_application::CommandEnvelope;
use forge_domain::{
    ArtifactId, LifecycleStatus,
    file_snapshot::{FileSnapshotManifest, FileSnapshotState},
};
use forge_protocol::wire::{CommandName, CommandRequest, CommandStatus};
use forge_storage::evidence::ObjectEvidenceStore;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS"]
async fn selected_files_are_immutable_task_artifacts_and_future_inputs_without_git() -> Result<()> {
    let runtime = private_tempdir()?;
    let source = tempfile::tempdir()?;
    let objects = Arc::new(object_store::memory::InMemory::new());
    let harness = M0Harness::start_configured(|core| {
        Ok(core
            .with_execution_root(runtime.path().to_owned())?
            .with_test_object_store(ObjectEvidenceStore::from_store(objects.clone()))?)
    })
    .await?;
    let project = harness.create_project("File snapshots").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let source_task = harness
        .create_task(project, pipeline, "Source files")
        .await?;
    let receiver = harness
        .create_task(project, pipeline, "Use selected inputs")
        .await?;
    fs::write(source.path().join("bytes.bin"), [0, 255, 1])?;
    fs::write(source.path().join("empty.txt"), [])?;
    let payload = json!({"task_id":source_task,"expected_task_revision":1,"title":"Selected files",
        "root":source.path(),"paths":["bytes.bin","empty.txt"]});
    let envelope = CommandEnvelope::parse(
        CommandName::ImportTaskFileSnapshot,
        CommandRequest {
            project_id: project.to_string(),
            expected_revision: harness
                .store
                .load_project(project)
                .await?
                .context("Project")?
                .revision(),
            payload: payload.as_object().unwrap().clone(),
        },
        "snapshot-import",
    )?;
    let receipt = harness.core.execute_command(envelope.clone()).await?;
    let replay = harness.core.execute_command(envelope).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(replay.resource, receipt.resource);
    let pending = harness.store.pending_file_snapshots().await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, FileSnapshotState::Pending);
    assert_eq!(
        harness
            .required_task(source_task)
            .await?
            .task
            .revision()
            .get(),
        1
    );
    assert_eq!(harness.core.file_snapshot_tick().await?, 1);
    let view = harness
        .core
        .read_file_snapshots(project, source_task, None, 50)
        .await?;
    assert_eq!(view.len(), 1);
    assert_eq!(view[0]["state"], "sealed");
    assert!(!serde_json::to_string(&view)?.contains(source.path().to_str().unwrap()));
    let artifact_id: ArtifactId = serde_json::from_value(view[0]["artifact_id"].clone())?;
    let manifest: FileSnapshotManifest = serde_json::from_value(view[0]["manifest"].clone())?;
    manifest.validate(artifact_id)?;
    assert_eq!(manifest.files[1].size_bytes, 0);
    fs::write(source.path().join("bytes.bin"), b"changed")?;
    let objects = ObjectEvidenceStore::from_store(objects);
    assert_eq!(
        objects.read_snapshot_file(&manifest.files[0]).await?,
        vec![0, 255, 1]
    );
    assert_eq!(harness.core.file_snapshot_tick().await?, 0);
    let task = harness.required_task(source_task).await?;
    assert_eq!(task.task.lifecycle(), LifecycleStatus::Draft);
    assert_eq!(task.task.artifact_links().len(), 1);
    harness
        .execute(
            project,
            CommandName::AttachTaskFileInput,
            json!({"task_id":receiver,"expected_task_revision":1,"artifact_id":artifact_id}),
        )
        .await?;
    let mut tx = harness.store.begin().await?;
    let inputs = tx.task_file_inputs(project, receiver).await?;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].manifest, manifest);
    tx.commit().await?;
    let router = forge_core::router(harness.core.clone());
    let snapshots_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{project}/tasks/{source_task}/file-snapshots"
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(snapshots_response.status(), StatusCode::OK);
    let snapshots: Value =
        serde_json::from_slice(&to_bytes(snapshots_response.into_body(), 64 * 1024).await?)?;
    assert_eq!(
        snapshots["items"][0]["manifest"]["files"][0]["sha256"],
        manifest.files[0].sha256
    );
    assert!(
        snapshots["items"][0]["manifest"]["files"][0]
            .get("object_key")
            .is_none()
    );
    let inputs_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{project}/tasks/{receiver}/file-inputs"
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(inputs_response.status(), StatusCode::OK);
    let input_read: Value =
        serde_json::from_slice(&to_bytes(inputs_response.into_body(), 64 * 1024).await?)?;
    assert_eq!(input_read["items"][0]["artifact_id"], json!(artifact_id));
    assert_eq!(
        input_read["items"][0]["manifest"],
        snapshots["items"][0]["manifest"]
    );
    assert!(!serde_json::to_string(&input_read)?.contains("object_key"));
    let foreign_response = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{project}/tasks/{}/file-snapshots",
                    forge_domain::TaskId::new()
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(foreign_response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        harness.required_task(receiver).await?.task.revision().get(),
        1
    );
    assert_eq!(
        harness.required_task(receiver).await?.task.lifecycle(),
        LifecycleStatus::Draft
    );
    let foreign = harness.create_project("Other Project").await?;
    let foreign_pipeline = harness
        .create_pipeline(foreign, single_stage_pipeline())
        .await?;
    let other = harness
        .create_task(foreign, foreign_pipeline, "Other Task")
        .await?;
    assert!(
        harness
            .execute(
                foreign,
                CommandName::AttachTaskFileInput,
                json!({"task_id":other,"expected_task_revision":1,"artifact_id":artifact_id})
            )
            .await
            .is_err()
    );
    assert!(harness.execute(project,CommandName::AttachTaskFileInput,json!({"task_id":source_task,"expected_task_revision":task.task.revision().get(),"artifact_id":artifact_id})).await.is_err());
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS"]
async fn unsafe_source_fails_without_an_artifact_and_capture_never_stops_a_run() -> Result<()> {
    let runtime = private_tempdir()?;
    let source = tempfile::tempdir()?;
    let harness = M0Harness::start_configured(|core| {
        Ok(core
            .with_execution_root(runtime.path().to_owned())?
            .with_test_object_store(ObjectEvidenceStore::from_store(Arc::new(
                object_store::memory::InMemory::new(),
            )))?)
    })
    .await?;
    let project = harness.create_project("Unsafe snapshot").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "Unstarted Task")
        .await?;
    fs::write(source.path().join("real"), b"input")?;
    std::os::unix::fs::symlink("real", source.path().join("link"))?;
    harness.execute(project,CommandName::ImportTaskFileSnapshot,json!({"task_id":task,"expected_task_revision":1,"title":"Unsafe link","root":source.path(),"paths":["link"]})).await?;
    assert_eq!(harness.core.file_snapshot_tick().await?, 1);
    let view = harness
        .core
        .read_file_snapshots(project, task, None, 50)
        .await?;
    assert_eq!(view[0]["state"], "failed");
    assert_eq!(
        harness
            .required_task(task)
            .await?
            .task
            .artifact_links()
            .len(),
        0
    );
    let before = harness.required_task(task).await?.task;
    assert!(harness.execute(project,CommandName::CaptureTaskFileSnapshot,json!({"task_id":task,"expected_task_revision":1,"title":"Capture","paths":["real"]})).await.is_err());
    assert_eq!(harness.required_task(task).await?.task, before);
    harness.shutdown().await;
    Ok(())
}

#[test]
fn snapshot_commands_reject_implicit_paths_and_credentials_before_any_io() {
    use forge_domain::{ProjectId, TaskId};
    let parse = |payload: Value| {
        CommandEnvelope::parse(
            CommandName::ImportTaskFileSnapshot,
            CommandRequest {
                project_id: ProjectId::new().to_string(),
                expected_revision: 1,
                payload: payload.as_object().unwrap().clone(),
            },
            "input-validation",
        )
    };
    for (root, path) in [
        ("/tmp/input", "../escape"),
        ("/tmp/input", ".git/config"),
        ("/tmp/input", "*"),
        ("/home/operator/.codex", "auth.json"),
        ("/", "file"),
    ] {
        assert!(parse(json!({"task_id":TaskId::new(),"expected_task_revision":1,"title":"Files","root":root,"paths":[path]})).is_err());
    }
}

fn private_tempdir() -> Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    Ok(directory)
}

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS"]
async fn parent_import_cannot_read_runtime_grants_or_a_separate_secret_store() -> Result<()> {
    use forge_provider_common::{MasterKeySource, SecretStore};
    let parent = private_tempdir()?;
    let runtime = parent.path().join("execution");
    fs::create_dir(&runtime)?;
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    fs::write(runtime.join("synthetic-token"), b"must-not-be-exported")?;
    let secrets =
        SecretStore::initialize(&parent.path().join("keys"), MasterKeySource::OwnerOnlyFile)?;
    let harness = M0Harness::start_configured(|core| {
        Ok(core
            .with_secret_store(secrets)
            .with_execution_root(runtime)?
            .with_test_object_store(ObjectEvidenceStore::from_store(Arc::new(
                object_store::memory::InMemory::new(),
            )))?)
    })
    .await?;
    let project = harness.create_project("Import exclusions").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "No secret export")
        .await?;
    // Neither path contains a credential filename prohibited by input parsing.
    for path in ["execution/synthetic-token", "keys/master-key-binding.json"] {
        harness
            .execute(
                project,
                CommandName::ImportTaskFileSnapshot,
                json!({"task_id":task,"expected_task_revision":1,
            "title":"Protected file","root":parent.path(),"paths":[path]}),
            )
            .await?;
        assert_eq!(harness.core.file_snapshot_tick().await?, 1);
    }
    let operations = harness
        .core
        .read_file_snapshots(project, task, None, 50)
        .await?;
    assert_eq!(operations.len(), 2);
    assert!(
        operations
            .iter()
            .all(|op| op["state"] == "failed" && op["error_code"] == "unsafe_source")
    );
    assert!(
        harness
            .required_task(task)
            .await?
            .task
            .artifact_links()
            .is_empty()
    );
    assert!(
        fs::read_dir(parent.path().join("execution/file-captures"))?
            .next()
            .is_none()
    );
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS"]
async fn pending_surface_capture_blocks_integration_until_reads_are_finished() -> Result<()> {
    use forge_domain::{
        Actor, ActorId, Timestamp,
        file_snapshot::{FileCaptureRequest, FileSnapshotOperation, FileSnapshotSource},
    };
    use uuid::Uuid;
    let harness = M0Harness::start().await?;
    let project = harness.create_project("Capture admission").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "Reserved surface")
        .await?;
    harness.start_project(project).await?;
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(tx.integration_dispatch_open(project, task).await?);
    let id = Uuid::now_v7();
    let mut op = FileSnapshotOperation {
        id,
        artifact_id: ArtifactId::new(),
        project_id: project,
        task_id: task,
        title: "Quiescent capture".into(),
        source: FileSnapshotSource::TaskSurface {
            request: FileCaptureRequest {
                operation_id: id,
                project_id: project,
                task_id: task,
                run_id: Uuid::now_v7(),
                fencing_token: 1,
                environment_epoch: 1,
                surface_id: Uuid::now_v7(),
                paths: vec!["selected.txt".into()],
            },
        },
        state: FileSnapshotState::Pending,
        manifest: None,
        error_code: None,
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    };
    tx.insert_file_snapshot(&op).await?;
    assert!(tx.file_capture_is_busy(task).await?);
    assert!(!tx.integration_dispatch_open(project, task).await?);
    tx.commit().await?;
    // The guard persists across transactions/restart, not an in-memory cooldown.
    let mut tx = harness.store.begin().await?;
    tx.lock_project(project).await?;
    assert!(!tx.integration_dispatch_open(project, task).await?);
    op.state = FileSnapshotState::Failed;
    op.error_code = Some("completed_failure".into());
    tx.finish_file_snapshot(&op).await?;
    assert!(!tx.file_capture_is_busy(task).await?);
    assert!(tx.integration_dispatch_open(project, task).await?);
    tx.commit().await?;
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS"]
async fn pending_surface_captures_cannot_starve_a_later_local_import() -> Result<()> {
    use forge_domain::{
        Actor, ActorId, Timestamp,
        file_snapshot::{FileCaptureRequest, FileSnapshotOperation, FileSnapshotSource},
    };
    use uuid::Uuid;

    let runtime = private_tempdir()?;
    let source = tempfile::tempdir()?;
    fs::write(source.path().join("selected.txt"), b"independent import")?;
    let harness = M0Harness::start_configured(|core| {
        Ok(core
            .with_execution_root(runtime.path().to_owned())?
            .with_test_object_store(ObjectEvidenceStore::from_store(Arc::new(
                object_store::memory::InMemory::new(),
            )))?)
    })
    .await?;
    let project = harness.create_project("Fair snapshot queue").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    // Simulate durable capture requests left pending while the Supervisor is
    // disconnected. No runtime, inference, fabricated result or live files.
    let mut captures = Vec::new();
    for index in 0..32 {
        let task = harness
            .create_task(project, pipeline, &format!("Awaiting capture {index}"))
            .await?;
        let id = Uuid::now_v7();
        let op = FileSnapshotOperation {
            id,
            artifact_id: ArtifactId::new(),
            project_id: project,
            task_id: task,
            title: "Waiting for retained writer".into(),
            source: FileSnapshotSource::TaskSurface {
                request: FileCaptureRequest {
                    operation_id: id,
                    project_id: project,
                    task_id: task,
                    run_id: Uuid::now_v7(),
                    fencing_token: 1,
                    environment_epoch: 1,
                    surface_id: Uuid::now_v7(),
                    paths: vec!["selected.txt".into()],
                },
            },
            state: FileSnapshotState::Pending,
            manifest: None,
            error_code: None,
            created_by: Actor::human(ActorId::new()),
            created_at: Timestamp::now_utc(),
        };
        let mut tx = harness.store.begin().await?;
        tx.insert_file_snapshot(&op).await?;
        tx.commit().await?;
        captures.push(op);
    }
    let receiver = harness
        .create_task(project, pipeline, "Later independent import")
        .await?;
    harness
        .execute(
            project,
            CommandName::ImportTaskFileSnapshot,
            json!({"task_id":receiver,"expected_task_revision":1,
                "title":"Independent bytes","root":source.path(),"paths":["selected.txt"]}),
        )
        .await?;

    let initial = harness.store.pending_file_snapshots().await?;
    assert_eq!(
        initial, captures,
        "the first bounded page is entirely blocked"
    );
    assert_eq!(harness.core.file_snapshot_tick().await?, 0);
    let wrapped = harness
        .store
        .pending_file_snapshots_after(Some(captures[31].id))
        .await?;
    assert_eq!(wrapped.len(), 32);
    assert_eq!(wrapped[0].task_id, receiver);
    assert_eq!(wrapped[1].id, captures[0].id, "the page wraps by identity");
    assert_eq!(harness.core.file_snapshot_tick().await?, 1);
    let imported = harness
        .core
        .read_file_snapshots(project, receiver, None, 50)
        .await?;
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0]["state"], "sealed");
    assert_eq!(harness.store.pending_file_snapshots().await?, captures);
    assert_eq!(
        harness
            .required_task(receiver)
            .await?
            .task
            .artifact_links()
            .len(),
        1
    );
    // The cursor also advances before errors: a corrupt earlier failure receipt
    // stays diagnosable but cannot monopolize every subsequent tick.
    let failed = runtime
        .path()
        .join("file-captures")
        .join(format!("{}.failed", captures[31].id));
    fs::write(&failed, b"wrong capture scope")?;
    fs::set_permissions(&failed, fs::Permissions::from_mode(0o600))?;
    harness
        .execute(
            project,
            CommandName::ImportTaskFileSnapshot,
            json!({"task_id":receiver,
                "expected_task_revision":harness.required_task(receiver).await?.task.revision().get(),
                "title":"Import after bad receipt","root":source.path(),"paths":["selected.txt"]}),
        )
        .await?;
    assert!(harness.core.file_snapshot_tick().await.is_err());
    assert_eq!(harness.core.file_snapshot_tick().await?, 1);
    let imported = harness
        .core
        .read_file_snapshots(project, receiver, None, 50)
        .await?;
    assert_eq!(imported.len(), 2);
    assert!(
        imported
            .iter()
            .all(|operation| operation["state"] == "sealed")
    );
    assert_eq!(harness.store.pending_file_snapshots().await?, captures);
    harness.shutdown().await;
    Ok(())
}
