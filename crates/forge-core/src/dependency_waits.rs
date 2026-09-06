//! Supervisor outcomes reuse the command engine's dependency reconciliation.

use forge_application::engine::Engine;
use forge_domain::{CommandId, DomainEvent, Project, TaskId, Timestamp};
use forge_storage::StorageTransaction;

use crate::{CoreError, CoreService};

impl CoreService {
    pub(crate) async fn resolve_dependency_waits_after_completion(
        &self,
        transaction: &mut StorageTransaction<'_>,
        project: &mut Project,
        blocker_task_id: TaskId,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, CoreError> {
        Engine::new(
            &self.command_context(project.id()),
            self.command_clock.as_ref(),
        )
        .resolve_dependency_waits_after_completion(
            transaction,
            project,
            blocker_task_id,
            command_id,
            now,
        )
        .await
        .map_err(Into::into)
    }

    pub(crate) async fn wait_dependents_for_unsatisfied_blocker(
        &self,
        transaction: &mut StorageTransaction<'_>,
        project: &mut Project,
        blocker_task_id: TaskId,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, CoreError> {
        Engine::new(
            &self.command_context(project.id()),
            self.command_clock.as_ref(),
        )
        .wait_dependents_for_unsatisfied_blocker(
            transaction,
            project,
            blocker_task_id,
            command_id,
            now,
        )
        .await
        .map_err(Into::into)
    }
}
