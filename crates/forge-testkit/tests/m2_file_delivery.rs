//! Real Core/PG/gRPC with a manual Supervisor and byte-exact in-memory object store.
//! Credentials, capture contents and observations are synthetic; no provider executes.

use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use anyhow::{Context, Result};
use forge_domain::{
    ArtifactId, CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, LifecycleStatus,
    PipelineVersionId, ProjectId, TaskId,
    file_snapshot::{FileSnapshotSource, FileSnapshotState, MAX_SNAPSHOT_BYTES, TaskFileInput},
    runtime::{RuntimeBinding, RuntimeLaunchSpec, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, RunEventKind},
    wire::CommandName,
};
use forge_provider_claude::ClaudeAdapter;
use forge_provider_common::{
    MasterKeySource, PrivateMaterialization, SecretBytes, SecretStore,
    selected_file::capture_selected_files,
};
use forge_storage::evidence::ObjectEvidenceStore;
use forge_testkit::m0::{M0Harness, ManualSupervisor, single_stage_pipeline};
use serde_json::json;
use uuid::Uuid;

struct Setup {
    root: tempfile::TempDir,
    source: tempfile::TempDir,
    harness: M0Harness,
    supervisor: ManualSupervisor,
    project: ProjectId,
    version: PipelineVersionId,
}

fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn private_tempdir() -> Result<tempfile::TempDir> {
    let root = tempfile::tempdir()?;
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))?;
    Ok(root)
}

async fn setup() -> Result<Setup> {
    let root = private_tempdir()?;
    let source = private_tempdir()?;
    let secrets =
        SecretStore::initialize(&root.path().join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let execution = root.path().to_owned();
    let harness = M0Harness::start_configured(move |core| {
        Ok(core
            .with_secret_store(secrets)
            .with_execution_root(execution)?
            .with_test_object_store(ObjectEvidenceStore::from_store(Arc::new(
                object_store::memory::InMemory::new(),
            )))?)
    })
    .await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let project = harness.create_project("Selected-file delivery").await?;
    let secret_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    let token = PrivateMaterialization::create(
        &root.path().join("synthetic-token"),
        &SecretBytes::new(b"sk-ant-oat01-synthetic-no-inference".to_vec()),
    )?;
    harness
        .execute(
            project,
            CommandName::EnrollCredential,
            json!({"secret_id":secret_id,
        "binding_id":binding_id,"kind":"claude_subscription","source_file":token.path()}),
        )
        .await?;
    harness
        .create_employee(project, "File-input Employee")
        .await?;
    let employee = harness
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    let delivery = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let binding = RuntimeBinding {
        execution_profile: ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: project,
            adapter_id: "claude_code_cli".into(),
            adapter_version: "2.1.263".into(),
            provider_id: "anthropic".into(),
            model: "no-inference".into(),
            credential_binding: CredentialBinding {
                id: binding_id,
                project_id: project,
                secret_id,
                account_id: Some("fixture".into()),
                allowed_delivery_modes: BTreeSet::from([delivery]),
            },
            credential_delivery: delivery,
            capability_profile: ClaudeAdapter::capabilities(),
        }
        .try_into()?,
        image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
        surface: SurfaceSpec::FilesystemSandbox,
        access: SurfaceAccess::ReadWrite,
        limits: Default::default(),
        budget: Default::default(),
        system_prompt: "Synthetic rules".into(),
        employee_prompt: "Synthetic Employee".into(),
    };
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":employee,"binding":binding}),
        )
        .await?;
    let version = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    Ok(Setup {
        root,
        source,
        harness,
        supervisor,
        project,
        version,
    })
}

async fn import(setup: &Setup, task: TaskId, paths: &[&str]) -> Result<TaskFileInput> {
    let revision = setup
        .harness
        .required_task(task)
        .await?
        .task
        .revision()
        .get();
    let operation = setup
        .harness
        .execute(
            setup.project,
            CommandName::ImportTaskFileSnapshot,
            json!({"task_id":task,"expected_task_revision":revision,"title":"Selected input",
            "root":setup.source.path(),"paths":paths}),
        )
        .await?
        .resource
        .context("operation")?
        .id;
    assert_eq!(setup.harness.core.file_snapshot_tick().await?, 1);
    let views = setup
        .harness
        .core
        .read_file_snapshots(setup.project, task, None, 50)
        .await?;
    let view = views
        .iter()
        .find(|view| view["id"] == json!(operation))
        .context("sealed operation")?;
    assert_eq!(view["state"], "sealed");
    Ok(TaskFileInput {
        artifact_id: serde_json::from_value(view["artifact_id"].clone())?,
        manifest: serde_json::from_value(view["manifest"].clone())?,
    })
}

async fn attach(setup: &Setup, receiver: TaskId, artifact: ArtifactId) -> Result<()> {
    let revision = setup
        .harness
        .required_task(receiver)
        .await?
        .task
        .revision()
        .get();
    setup
        .harness
        .execute(
            setup.project,
            CommandName::AttachTaskFileInput,
            json!({"task_id":receiver,"expected_task_revision":revision,"artifact_id":artifact}),
        )
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicitly configured local PostgreSQL and NATS; no provider"]
async fn immutable_inputs_reach_future_runs_and_capture_reserves_a_quiescent_surface() -> Result<()>
{
    let setup = Box::pin(setup()).await?;
    Box::pin(exercise(setup)).await
}

async fn exercise(mut setup: Setup) -> Result<()> {
    let source_task = setup
        .harness
        .create_task(setup.project, setup.version, "Snapshot source")
        .await?;
    let receiver = setup
        .harness
        .create_task(setup.project, setup.version, "Snapshot receiver")
        .await?;
    fs::write(setup.source.path().join("binary.bin"), [0, 255, 1])?;
    fs::write(setup.source.path().join("empty.txt"), [])?;
    fs::write(setup.source.path().join("tool.sh"), b"#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(
        setup.source.path().join("tool.sh"),
        fs::Permissions::from_mode(0o700),
    )?;
    let first_input = Box::pin(import(
        &setup,
        source_task,
        &["binary.bin", "empty.txt", "tool.sh"],
    ))
    .await?;
    Box::pin(attach(&setup, receiver, first_input.artifact_id)).await?;
    setup.harness.approve_task(setup.project, receiver).await?;
    setup.harness.start_project(setup.project).await?;
    let provision = setup.supervisor.next_provision_for_task(receiver).await?;
    let run = setup
        .harness
        .wait_for_run_count(receiver, 1)
        .await?
        .remove(0);
    let spec: RuntimeLaunchSpec = serde_json::from_str(&provision.run_spec_json)?;
    assert_eq!(provision.run_spec_version, 6);
    assert_eq!(spec.schema_version, 6);
    assert_eq!(spec.file_inputs, vec![first_input.clone()]);
    assert_eq!(run.run_spec["file_inputs"], json!([first_input]));
    assert!(
        spec.source_request.is_none(),
        "selected files do not introduce Git"
    );
    let epoch = setup
        .root
        .path()
        .join("file-inputs")
        .join(run.id.to_string())
        .join(run.environment_epoch.to_string());
    let materialized = epoch
        .join(first_input.artifact_id.to_string())
        .join("files");
    assert_eq!(fs::read(materialized.join("binary.bin"))?, vec![0, 255, 1]);
    assert_eq!(fs::read(materialized.join("empty.txt"))?, Vec::<u8>::new());
    assert_eq!(
        fs::metadata(materialized.join("tool.sh"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        serde_json::from_slice::<Vec<TaskFileInput>>(&fs::read(epoch.join("inputs.json"))?)?,
        spec.file_inputs
    );
    assert!(
        !setup.root.path().join("surfaces").exists(),
        "Core input delivery does not overlay or create a Task workspace"
    );

    let running = setup
        .supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Running)
        .await?;
    assert_eq!(
        running.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let before = setup.harness.required_task(receiver).await?.task;
    let before_run = setup.harness.store.list_runs(receiver).await?.remove(0);
    assert_eq!(before.lifecycle(), LifecycleStatus::InProgress);
    assert_eq!(
        before_run.desired_state,
        forge_storage::RunDesiredState::Running
    );
    assert_eq!(
        before_run.observed_state,
        forge_storage::RunObservedState::Running
    );
    assert!(setup.harness.execute(setup.project,CommandName::CaptureTaskFileSnapshot,
        json!({"task_id":receiver,"expected_task_revision":before.revision().get(),"title":"Not an implicit stop","paths":["result.bin"]})).await.is_err());
    assert_eq!(setup.harness.required_task(receiver).await?.task, before);
    assert!(
        setup
            .harness
            .store
            .pending_file_snapshots()
            .await?
            .is_empty()
    );
    let unchanged_run = setup.harness.store.list_runs(receiver).await?.remove(0);
    assert_eq!(unchanged_run.id, before_run.id);
    assert_eq!(unchanged_run.desired_state, before_run.desired_state);
    assert_eq!(unchanged_run.observed_state, before_run.observed_state);
    assert_eq!(unchanged_run.last_sequence, before_run.last_sequence);
    assert_eq!(unchanged_run.observed_details, before_run.observed_details);
    assert_eq!(
        unchanged_run.lease_fencing_token,
        before_run.lease_fencing_token
    );
    assert_eq!(
        unchanged_run.environment_epoch,
        before_run.environment_epoch
    );
    assert_eq!(unchanged_run.run_spec, before_run.run_spec);

    // Attach a second Artifact during execution. Only a future Run can see it.
    fs::write(setup.source.path().join("later.txt"), b"future Run only")?;
    let later = Box::pin(import(&setup, source_task, &["later.txt"])).await?;
    Box::pin(attach(&setup, receiver, later.artifact_id)).await?;
    assert_eq!(
        setup
            .harness
            .store
            .list_runs(receiver)
            .await?
            .remove(0)
            .run_spec,
        run.run_spec
    );
    assert!(!epoch.join(later.artifact_id.to_string()).exists());
    Box::pin(capture_and_resume(
        &mut setup,
        receiver,
        run,
        spec,
        first_input,
        later,
    ))
    .await?;
    setup.harness.stop_project(setup.project).await?;
    setup.harness.shutdown().await;
    Ok(())
}

async fn capture_and_resume(
    setup: &mut Setup,
    task: TaskId,
    run: forge_storage::RunProjection,
    spec: RuntimeLaunchSpec,
    first: TaskFileInput,
    later: TaskFileInput,
) -> Result<()> {
    let revision = setup
        .harness
        .required_task(task)
        .await?
        .task
        .revision()
        .get();
    setup
        .harness
        .execute(
            setup.project,
            CommandName::PauseTask,
            json!({"task_id":task,"expected_task_revision":revision,"mode":"graceful"}),
        )
        .await?;
    setup
        .supervisor
        .next_stop_for_run(&run.id.to_string())
        .await?;
    let stopped = setup
        .supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Stopped)
        .await?;
    assert_eq!(
        stopped.disposition,
        AcknowledgementDisposition::Accepted as i32
    );
    let paused = setup.harness.required_task(task).await?.task;
    assert_eq!(paused.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(
        paused.resume_to_lifecycle(),
        Some(forge_domain::ResumeLifecycleStatus::InProgress)
    );
    let operation = setup.harness.execute(setup.project,CommandName::CaptureTaskFileSnapshot,
        json!({"task_id":task,"expected_task_revision":paused.revision().get(),"title":"Stopped output","paths":["result.bin"]})).await?.resource.context("capture operation")?.id;
    assert_eq!(
        setup.harness.required_task(task).await?.task,
        paused,
        "capture admission does not change lifecycle"
    );
    let mut pending = setup.harness.store.pending_file_snapshots().await?;
    assert_eq!(pending.len(), 1);
    let pending = pending.remove(0);
    assert_eq!(pending.id.to_string(), operation);
    assert_eq!(pending.state, FileSnapshotState::Pending);
    let FileSnapshotSource::TaskSurface { request } = &pending.source else {
        anyhow::bail!("Task capture expected")
    };
    assert_eq!(request.run_id, run.id);
    assert_eq!(request.surface_id, spec.surface_id);
    assert_eq!(request.fencing_token, run.lease_fencing_token);
    assert_eq!(request.environment_epoch, run.environment_epoch);
    assert_eq!(
        setup.harness.core.file_snapshot_tick().await?,
        0,
        "no capture completion is inferred from a stop"
    );
    let wait = paused
        .wait_conditions()
        .next()
        .context("manual pause")?
        .id();
    setup.harness.execute(setup.project,CommandName::ResumeTask,
        json!({"task_id":task,"expected_task_revision":paused.revision().get(),"wait_condition_id":wait})).await?;
    // Resume restores the lifecycle saved before PauseTask; this is not proof
    // that a replacement Run was admitted. Capture must still exclude writers.
    assert_eq!(
        setup.harness.required_task(task).await?.task.lifecycle(),
        LifecycleStatus::InProgress
    );
    assert_eq!(
        setup.harness.core.dispatch_available(setup.project).await?,
        0
    );
    assert_eq!(
        setup.harness.store.list_runs(task).await?.len(),
        1,
        "pending capture retains its maintenance reservation"
    );
    let remaining_run = setup.harness.store.list_runs(task).await?.remove(0);
    assert_eq!(remaining_run.id, run.id);
    assert_eq!(
        remaining_run.observed_state,
        forge_storage::RunObservedState::Stopped
    );
    let physical_or_lease_count: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM run_environment_reservations WHERE task_id=$1 AND released_at IS NULL) + (SELECT count(*) FROM leases WHERE task_id=$1 AND lease_state='active')",
    ).bind(task.as_uuid()).fetch_one(&setup.harness.pool).await?;
    assert_eq!(
        physical_or_lease_count, 0,
        "the pending capture, not an unreleased Run, blocks dispatch"
    );
    let mut tx = setup.harness.store.begin().await?;
    assert!(tx.file_capture_is_busy(task).await?);
    assert!(!tx.integration_dispatch_open(setup.project, task).await?);
    tx.commit().await?;

    // Synthetic Supervisor completion, with the exact durable request identity.
    // This exercises Core's cross-boundary intake, not physical Podman execution.
    let surfaces = setup.root.path().join("surfaces");
    private_directory(&surfaces)?;
    let surface = surfaces.join(spec.surface_id.to_string());
    private_directory(&surface)?;
    let worktree = surface.join("worktree");
    private_directory(&worktree)?;
    fs::write(worktree.join("result.bin"), [9, 0, 255])?;
    let target = setup
        .root
        .path()
        .join("file-captures")
        .join(pending.id.to_string());
    capture_selected_files(
        &worktree,
        &request.paths,
        &target,
        &serde_json::to_vec(request)?,
        MAX_SNAPSHOT_BYTES,
    )?;
    assert_eq!(setup.harness.core.file_snapshot_tick().await?, 1);
    let view = setup
        .harness
        .core
        .read_file_snapshots(setup.project, task, None, 50)
        .await?;
    assert_eq!(view[0]["state"], "sealed");
    assert_eq!(
        setup
            .harness
            .required_task(task)
            .await?
            .task
            .artifact_links()
            .len(),
        1
    );
    assert!(
        setup
            .harness
            .store
            .pending_file_snapshots()
            .await?
            .is_empty()
    );
    let next = setup.supervisor.next_provision_for_task(task).await?;
    assert_ne!(next.run_id, run.id.to_string());
    let next_spec: RuntimeLaunchSpec = serde_json::from_str(&next.run_spec_json)?;
    assert_eq!(next_spec.file_inputs.len(), 2);
    assert!(next_spec.file_inputs.contains(&first));
    assert!(next_spec.file_inputs.contains(&later));
    let next_input = setup
        .root
        .path()
        .join("file-inputs")
        .join(&next.run_id)
        .join(next.environment_epoch.to_string())
        .join(later.artifact_id.to_string())
        .join("files/later.txt");
    assert_eq!(fs::read(next_input)?, b"future Run only");
    assert_eq!(setup.harness.store.list_runs(task).await?.len(), 2);
    assert_ne!(
        setup.harness.required_task(task).await?.task.lifecycle(),
        LifecycleStatus::Done,
        "snapshot is not a stage verdict"
    );
    assert_eq!(fs::read(worktree.join("result.bin"))?, vec![9, 0, 255]);
    assert!(
        !worktree.join("later.txt").exists(),
        "even the next Run input remains separate from workspace"
    );
    Ok(())
}
