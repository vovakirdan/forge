//! Source registration is management intent, not host Git execution or verification.
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Project, ProjectRepository, Timestamp,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
use uuid::Uuid;

use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    task_support::{load_scoped_task, persist_task_and_project, require_task_revision, task_event},
};
use crate::{CommandEnvelope, CommandPayload};

impl Engine<'_> {
    pub(super) async fn manage_repository(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let (audit, primary) = match &envelope.payload {
            CommandPayload::RegisterProjectRepository {
                name,
                source,
                target_ref,
            } => {
                let repository = ProjectRepository {
                    id: Uuid::now_v7(),
                    project_id: project.id(),
                    name: name.clone(),
                    source: source.clone(),
                    target_ref: target_ref.clone(),
                    created_by: self.actors.human,
                    created_at: now,
                };
                repository.validate_snapshot()?;
                transaction.insert_project_repository(&repository).await?;
                let revision = project.revision();
                project.record_child_mutation(now)?;
                transaction.update_project(&project, revision).await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::ProjectRepositoryRegistered,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("repository_id", json!(repository.id)),
                        ("name", json!(repository.name)),
                        ("target_ref", json!(repository.target_ref)),
                    ]),
                    now,
                )?;
                (audit, resource("project_repository", repository.id))
            }
            CommandPayload::BindTaskGitRepository {
                task_id,
                expected_task_revision,
                repository_id,
                initial_base,
            } => {
                let mut stored = load_scoped_task(transaction, &project, *task_id).await?;
                require_task_revision(&stored.task, *expected_task_revision)?;
                let repository = transaction
                    .load_project_repository(*repository_id)
                    .await?
                    .filter(|repository| repository.project_id == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "project repository",
                    })?;
                if stored.persistence.task_work_surface_id.is_some()
                    || stored.persistence.attempt_count != 0
                {
                    return Err(CommandError::InvalidTransport {
                        field: "task.work_surface",
                        reason: "cannot replace a retained execution surface".into(),
                    });
                }
                let surface_id = Uuid::now_v7();
                stored.task.bind_git_repository(
                    &repository,
                    initial_base.clone(),
                    surface_id,
                    now,
                )?;
                stored.persistence.task_work_surface_id = Some(surface_id);
                persist_task_and_project(
                    transaction,
                    &mut project,
                    &stored.task,
                    stored.persistence,
                    *expected_task_revision,
                    now,
                )
                .await?;
                let audit = task_event(
                    &project,
                    &stored.task,
                    DomainEventKind::TaskGitRepositoryBound,
                    self.actors.human,
                    command_id,
                    now,
                    event_payload([
                        ("repository_id", json!(repository.id)),
                        ("initial_base", json!(initial_base)),
                        ("surface_id", json!(surface_id)),
                        ("target_ref", json!(repository.target_ref)),
                    ]),
                )?;
                (audit, resource("task", task_id.as_uuid()))
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(primary),
        )
        .await
    }
}
