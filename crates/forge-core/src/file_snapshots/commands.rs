use forge_application::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, ArtifactId, CommandId, DomainEventKind, LifecycleStatus, Project, Timestamp,
    file_snapshot::{
        FileCaptureRequest, FileSnapshotOperation, FileSnapshotSource, FileSnapshotState,
        MAX_TASK_FILE_INPUTS,
    },
};
use forge_protocol::wire::CommandReceipt;
use forge_storage::StorageTransaction;
use serde_json::json;
use uuid::Uuid;

use super::invalid;
use crate::{
    CoreError, CoreService,
    command::{finish_command, resource},
    event::{event, event_payload},
    task_support::load_scoped_task,
};

impl CoreService {
    pub(crate) async fn manage_file_snapshot(
        &self,
        tx: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        let (task_id, expected) = match &envelope.payload {
            CommandPayload::ImportTaskFileSnapshot(input) => {
                (input.task_id, input.expected_task_revision)
            }
            CommandPayload::CaptureTaskFileSnapshot(input) => {
                (input.task_id, input.expected_task_revision)
            }
            CommandPayload::AttachTaskFileInput(input) => {
                (input.task_id, input.expected_task_revision)
            }
            _ => return Err(invalid("not a file snapshot command")),
        };
        let stored = load_scoped_task(tx, &project, task_id).await?;
        forge_application::engine::task_support::require_task_revision(&stored.task, expected)?;
        if matches!(
            stored.task.lifecycle(),
            LifecycleStatus::Done | LifecycleStatus::Cancelled
        ) {
            return Err(invalid(
                "terminal Task cannot accept new file evidence or inputs",
            ));
        }
        let (resource_ref, kind, detail) =
            if let CommandPayload::AttachTaskFileInput(input) = &envelope.payload {
                let snapshot = tx
                    .file_input_from_artifact(project.id(), input.artifact_id)
                    .await?
                    .ok_or_else(|| invalid("snapshot is not sealed in this Project"))?;
                if snapshot.manifest.source_task_id == task_id {
                    return Err(invalid("file input must come from another Task"));
                }
                let current = tx.task_file_inputs(project.id(), task_id).await?;
                if current.len() >= MAX_TASK_FILE_INPUTS
                    && !current
                        .iter()
                        .any(|item| item.artifact_id == input.artifact_id)
                {
                    return Err(invalid("Task file input limit reached"));
                }
                tx.attach_file_input(project.id(), task_id, &snapshot)
                    .await?;
                (
                    resource("artifact", input.artifact_id.as_uuid()),
                    DomainEventKind::TaskFileInputAttached,
                    json!({"artifact_id":input.artifact_id,"effective":"future_runs"}),
                )
            } else {
                if self
                    .evidence
                    .as_ref()
                    .and_then(|evidence| evidence.object_store.as_ref())
                    .is_none()
                    || self.execution.is_none()
                {
                    return Err(invalid(
                        "file snapshots require configured execution root and object storage",
                    ));
                }
                let operation_id = Uuid::now_v7();
                let (title, source) = match &envelope.payload {
                    CommandPayload::ImportTaskFileSnapshot(input) => (
                        input.title.clone(),
                        FileSnapshotSource::LocalFiles {
                            root: input.root.clone(),
                            paths: input.paths.clone(),
                        },
                    ),
                    CommandPayload::CaptureTaskFileSnapshot(input) => {
                        if tx.file_capture_is_busy(task_id).await? {
                            return Err(invalid(
                                "stop the Task first and wait for physical quiescence",
                            ));
                        }
                        let run = tx
                            .latest_file_capture_writer(project.id(), task_id)
                            .await?
                            .ok_or_else(|| invalid("Task has no retained writer surface"))?;
                        let spec: forge_domain::runtime::RuntimeLaunchSpec =
                            serde_json::from_value(run.run_spec)
                                .map_err(|_| invalid("invalid retained writer specification"))?;
                        if spec.binding.access != forge_domain::runtime::SurfaceAccess::ReadWrite
                            || spec.binding.surface == forge_domain::runtime::SurfaceSpec::None
                        {
                            return Err(invalid("Task has no writable source surface"));
                        }
                        (
                            input.title.clone(),
                            FileSnapshotSource::TaskSurface {
                                request: FileCaptureRequest {
                                    operation_id,
                                    project_id: project.id(),
                                    task_id,
                                    run_id: run.id,
                                    fencing_token: run.lease_fencing_token,
                                    environment_epoch: run.environment_epoch,
                                    surface_id: spec.surface_id,
                                    paths: input.paths.clone(),
                                },
                            },
                        )
                    }
                    _ => return Err(invalid("not a capture request")),
                };
                let op = FileSnapshotOperation {
                    id: operation_id,
                    artifact_id: ArtifactId::new(),
                    project_id: project.id(),
                    task_id,
                    title,
                    source,
                    state: FileSnapshotState::Pending,
                    manifest: None,
                    error_code: None,
                    created_by: self.actors.human,
                    created_at: now,
                };
                tx.insert_file_snapshot(&op).await?;
                (
                    resource("file_snapshot", op.id),
                    DomainEventKind::TaskFileSnapshotChanged,
                    json!({"operation_id":op.id,"artifact_id":op.artifact_id,"state":"pending"}),
                )
            };
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Task(task_id),
            stored.task.revision().get(),
            kind,
            self.actors.human,
            command_id,
            None,
            event_payload([("task_id", json!(task_id)), ("snapshot", detail)]),
            now,
        )?;
        finish_command(
            tx,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource_ref),
        )
        .await
    }
}
