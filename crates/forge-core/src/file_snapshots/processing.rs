use std::path::{Path, PathBuf};

use forge_domain::{
    AggregateRef, Artifact, ArtifactBody, ArtifactKind, ArtifactProducer, CommandId,
    DomainEventKind, LifecycleStatus, NewArtifact, Timestamp,
    file_snapshot::{
        FILE_SNAPSHOT_KIND, FileSnapshotManifest, FileSnapshotOperation, FileSnapshotSource,
        FileSnapshotState, MAX_SNAPSHOT_BYTES, SnapshotFile, snapshot_object_key,
    },
};
use forge_protocol::supervisor::v1::{CaptureFileSnapshot, CoreToSupervisor, core_to_supervisor};
use forge_provider_common::{
    PrivateMaterialization,
    selected_file::{CapturedFile, capture_selected_files, read_capture},
};
use forge_storage::ArtifactLocation;
use serde_json::json;

use super::invalid;
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    runtime_preparation::secure_directory,
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project, pinned_pipeline_version,
    },
};

impl CoreService {
    /// Drains committed capture/import intents. No provider inference is involved.
    pub async fn file_snapshot_tick(&self) -> Result<usize, CoreError> {
        let Ok(mut cursor) = self.file_snapshot_serial.try_lock() else {
            return Ok(0);
        };
        let (Some(execution), Some(objects)) = (
            &self.execution,
            self.evidence
                .as_ref()
                .and_then(|evidence| evidence.object_store.as_ref()),
        ) else {
            return Ok(0);
        };
        let root = execution.root.join("file-captures");
        secure_directory(&root)?;
        let mut completed = 0;
        for op in self.store.pending_file_snapshots_after(*cursor).await? {
            // Rotate before processing: even a failed receipt or unavailable
            // Supervisor must not monopolize later bounded passes.
            *cursor = Some(op.id);
            let target = root.join(op.id.to_string());
            let request = capture_identity(&op)?;
            let captured = match &op.source {
                FileSnapshotSource::LocalFiles {
                    root: source,
                    paths,
                } => {
                    let source = PathBuf::from(source);
                    // Local import cannot read Forge runtime grants through an alias.
                    if unsafe_import_source(
                        &source,
                        paths,
                        &execution.root,
                        self.secret_store.as_deref(),
                    ) || source.canonicalize().ok().as_ref() != Some(&source)
                    {
                        self.finish_file_capture(op, None, Some("unsafe_source"))
                            .await?;
                        completed += 1;
                        continue;
                    }
                    let (paths, target, request) = (paths.clone(), target.clone(), request.clone());
                    match tokio::task::spawn_blocking(move || {
                        capture_selected_files(
                            &source,
                            &paths,
                            &target,
                            &request,
                            MAX_SNAPSHOT_BYTES,
                        )
                    })
                    .await
                    {
                        Ok(Ok(files)) => Some(files),
                        _ => {
                            self.finish_file_capture(op, None, Some("capture_failed"))
                                .await?;
                            completed += 1;
                            continue;
                        }
                    }
                }
                FileSnapshotSource::TaskSurface { request: capture } => {
                    // Only the Supervisor publishes this completion marker after all
                    // reads have ended; incomplete staging never releases a surface.
                    let failed = root.join(format!("{}.failed", op.id));
                    if failed
                        .try_exists()
                        .map_err(|_| invalid("capture status unavailable"))?
                    {
                        let marker = PrivateMaterialization::open(&failed)
                            .and_then(|file| file.read(128 * 1024))
                            .map_err(|_| invalid("unsafe capture failure receipt"))?;
                        if marker.expose() != request {
                            return Err(invalid("capture failure scope mismatch"));
                        }
                        self.finish_file_capture(op, None, Some("capture_failed"))
                            .await?;
                        completed += 1;
                        continue;
                    }
                    // A partially written descriptor/request is not a completed failure.
                    let captured = read_capture(&target, &request, MAX_SNAPSHOT_BYTES)
                        .ok()
                        .flatten();
                    if captured.is_none() && self.supervisor.is_reconciled().await {
                        let request_json = serde_json::to_string(capture)
                            .map_err(|_| invalid("invalid capture request"))?;
                        self.supervisor
                            .send(CoreToSupervisor {
                                message: Some(core_to_supervisor::Message::CaptureFileSnapshot(
                                    CaptureFileSnapshot { request_json },
                                )),
                            })
                            .await?;
                    }
                    captured
                }
            };
            let Some(captured) = captured else {
                continue;
            };
            let manifest = manifest(&op, &captured)?;
            for file in &manifest.files {
                let bytes = PrivateMaterialization::read_beneath(
                    &target,
                    &Path::new("blobs").join(&file.sha256),
                    file.size_bytes,
                )
                .map_err(|_| invalid("captured file integrity check failed"))?;
                objects
                    .upload_snapshot_file(file, bytes.expose().to_vec())
                    .await
                    .map_err(|_| {
                        invalid("snapshot object storage unavailable; capture retained")
                    })?;
            }
            self.finish_file_capture(op, Some(manifest), None).await?;
            completed += 1;
        }
        Ok(completed)
    }

    async fn finish_file_capture(
        &self,
        original: FileSnapshotOperation,
        manifest: Option<FileSnapshotManifest>,
        error: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut tx = self.store.begin().await?;
        let mut project = tx
            .lock_project(original.project_id)
            .await?
            .ok_or_else(|| invalid("Project not found"))?;
        let mut stored = load_scoped_task(&mut tx, &project, original.task_id).await?;
        let Some(mut op) = tx.lock_file_snapshot(original.id).await? else {
            return Err(invalid("snapshot request not found"));
        };
        if op.state != FileSnapshotState::Pending {
            return Ok(());
        }
        if op != original {
            return Err(invalid("snapshot request changed"));
        }
        let now = crate::canonical_clock::project_mutation_time_at(&project, Timestamp::now_utc());
        let command_id = CommandId::new();
        let terminal = matches!(
            stored.task.lifecycle(),
            LifecycleStatus::Done | LifecycleStatus::Cancelled
        );
        op.error_code = error
            .map(str::to_owned)
            .or_else(|| terminal.then(|| "task_closed_before_capture_completed".into()));
        if op.error_code.is_none() {
            let manifest = manifest.ok_or_else(|| invalid("missing capture manifest"))?;
            manifest.validate(op.artifact_id)?;
            let artifact = Artifact::new(
                op.artifact_id,
                NewArtifact {
                    project_id: project.id(),
                    kind: ArtifactKind::new(FILE_SNAPSHOT_KIND)?,
                    title: op.title.clone(),
                    body: ArtifactBody::inline_json(
                        serde_json::to_value(&manifest)
                            .map_err(|_| invalid("invalid snapshot manifest"))?,
                    ),
                    metadata: json!({"operation_id":op.id}),
                    created_by: op.created_by,
                    created_at: now,
                },
            )?;
            let previous = stored.task.revision().get();
            stored.task.attach_artifact(
                &artifact,
                ArtifactProducer::Human,
                op.created_by,
                None,
                now,
            )?;
            tx.insert_artifact(
                &artifact,
                &ArtifactLocation {
                    task_id: Some(op.task_id),
                    run_id: None,
                    stage_id: stored.task.current_stage_id().cloned(),
                    producer: ArtifactProducer::Human,
                    producer_id: Some(op.id),
                    producer_data: json!({"operation_id":op.id}),
                },
            )
            .await?;
            persist_task_and_project(
                &mut tx,
                &mut project,
                &stored.task,
                stored.persistence,
                previous,
                now,
            )
            .await?;
            if stored.task.current_stage_id().is_some() {
                let version = pinned_pipeline_version(&mut tx, &project, &stored.task).await?;
                let executor = current_stage_executor(&version, &stored.task)?;
                forge_application::engine::scheduler::enqueue_if_employee(
                    &mut tx,
                    &project,
                    &stored.task,
                    &stored.persistence,
                    executor,
                    now,
                )
                .await?;
            }
            op.state = FileSnapshotState::Sealed;
            op.manifest = Some(manifest);
            tx.append_event_and_outbox(&event(
                project.id(),
                AggregateRef::Artifact(artifact.id()),
                1,
                DomainEventKind::ArtifactCreated,
                self.actors.core,
                command_id,
                None,
                event_payload([("kind", json!(FILE_SNAPSHOT_KIND))]),
                now,
            )?)
            .await?;
            tx.append_event_and_outbox(&event(
                project.id(),
                AggregateRef::Task(op.task_id),
                stored.task.revision().get(),
                DomainEventKind::TaskArtifactAttached,
                self.actors.core,
                command_id,
                None,
                event_payload([
                    ("artifact_id", json!(artifact.id())),
                    ("operation_id", json!(op.id)),
                ]),
                now,
            )?)
            .await?;
        } else {
            op.state = FileSnapshotState::Failed;
            let previous = project.revision();
            project.record_child_mutation(now)?;
            tx.update_project(&project, previous).await?;
        }
        tx.finish_file_snapshot(&op).await?;
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Task(op.task_id),
            stored.task.revision().get(),
            DomainEventKind::TaskFileSnapshotChanged,
            self.actors.core,
            command_id,
            None,
            event_payload([
                ("operation_id", json!(op.id)),
                ("artifact_id", json!(op.artifact_id)),
                ("state", json!(op.state)),
                ("error_code", json!(op.error_code)),
            ]),
            now,
        )?)
        .await?;
        tx.commit().await?;
        self.dispatch_after_command(project.id()).await;
        Ok(())
    }
}

fn unsafe_import_source(
    source: &Path,
    paths: &[String],
    execution_root: &Path,
    secrets: Option<&forge_provider_common::SecretStore>,
) -> bool {
    let protected = |path: &Path| {
        path.starts_with(execution_root) || secrets.is_some_and(|store| store.contains_path(path))
    };
    protected(source) || paths.iter().any(|path| protected(&source.join(path)))
}

fn capture_identity(op: &FileSnapshotOperation) -> Result<Vec<u8>, CoreError> {
    Ok(match &op.source {
        FileSnapshotSource::TaskSurface { request } => {
            serde_json::to_vec(request).map_err(|_| invalid("invalid capture request"))?
        }
        source => serde_json::to_vec(source).map_err(|_| invalid("invalid capture request"))?,
    })
}

fn manifest(
    op: &FileSnapshotOperation,
    captured: &[CapturedFile],
) -> Result<FileSnapshotManifest, CoreError> {
    let files = captured
        .iter()
        .map(|file| SnapshotFile {
            path: file.path.clone(),
            size_bytes: file.size_bytes,
            executable: file.executable,
            sha256: file.sha256.clone(),
            object_key: snapshot_object_key(op.project_id, op.artifact_id, &file.sha256),
        })
        .collect();
    let manifest = FileSnapshotManifest {
        schema_version: 1,
        project_id: op.project_id,
        source_task_id: op.task_id,
        files,
    };
    manifest.validate(op.artifact_id)?;
    let requested = match &op.source {
        FileSnapshotSource::LocalFiles { paths, .. } => paths,
        FileSnapshotSource::TaskSurface { request } => &request.paths,
    };
    if manifest
        .files
        .iter()
        .map(|file| &file.path)
        .ne(requested.iter())
    {
        return Err(invalid("capture selected paths mismatch"));
    }
    Ok(manifest)
}
