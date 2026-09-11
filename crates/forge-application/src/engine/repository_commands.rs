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
            CommandPayload::SetTaskGitSourcePolicy {
                task_id,
                expected_policy_revision,
                policy,
                reason,
            } => {
                if !matches!(
                    self.actors.human.kind(),
                    forge_domain::ActorKind::Human | forge_domain::ActorKind::SystemManager
                ) {
                    return Err(CommandError::Forbidden);
                }
                let stored = load_scoped_task(transaction, &project, *task_id).await?;
                let forge_domain::TaskWorkSurface::Git(binding) = stored.task.work_surface() else {
                    return Err(CommandError::InvalidTransport {
                        field: "task.work_surface",
                        reason: "a Task-owned Git binding is required".into(),
                    });
                };
                if let forge_domain::git::TaskGitSourcePolicy::PinnedCommit { commit } = policy
                    && forge_domain::git::GitObjectFormat::for_object(commit)
                        != binding.initial_base.object_format()
                {
                    return Err(CommandError::InvalidTransport {
                        field: "policy.commit",
                        reason: "object format differs from Task source".into(),
                    });
                }
                let current = transaction
                    .task_git_source_setting(project.id(), *task_id)
                    .await?;
                if current.revision != *expected_policy_revision {
                    return Err(super::RepositoryError::StaleRevision {
                        aggregate: "Task Git source policy",
                    }
                    .into());
                }
                let setting = current.changed(policy.clone()).map_err(|_| {
                    CommandError::InvalidTransport {
                        field: "policy.revision",
                        reason: "source policy revision exhausted".into(),
                    }
                })?;
                transaction
                    .insert_task_git_source_setting(project.id(), *task_id, &setting)
                    .await?;
                let revision = project.revision();
                project.record_child_mutation(now)?;
                transaction.update_project(&project, revision).await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Task(*task_id),
                    stored.task.revision().get(),
                    DomainEventKind::TaskGitSourcePolicyChanged,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("policy_revision", json!(setting.revision)),
                        ("policy", json!(setting.policy)),
                        ("reason", json!(reason)),
                    ]),
                    now,
                )?;
                (audit, resource("task_git_source_policy", task_id.as_uuid()))
            }
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
