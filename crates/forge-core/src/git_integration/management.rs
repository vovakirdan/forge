//! Named recovery permissions never stand in for a fresh target observation.
use super::*;
use forge_application::{CommandEnvelope, CommandPayload};
use forge_protocol::wire::CommandReceipt;

impl CoreService {
    pub(crate) async fn manage_git_integration(
        &self,
        tx: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        _now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        let (id, expected, reason, resolution) = match &envelope.payload {
            CommandPayload::RetryGitIntegration {
                operation_id,
                expected_task_revision,
                reason,
            } => (*operation_id, *expected_task_revision, reason, "retry"),
            CommandPayload::AcceptGitIntegrationResult {
                operation_id,
                expected_task_revision,
                reason,
            } => (*operation_id, *expected_task_revision, reason, "accept"),
            _ => return Err(invalid("unsupported integration command")),
        };
        let mut stored = tx
            .load_git_integration(project.id(), id)
            .await?
            .ok_or_else(|| invalid("integration absent"))?;
        let task = load_scoped_task(tx, &project, stored.operation.task_id)
            .await?
            .task;
        if stored.state != IntegrationState::Held
            || task.revision().get() != expected
            || !stored.operation.matches_task(&task)
            || task.lifecycle() != LifecycleStatus::InProgress
            || !tx
                .integration_dispatch_open(project.id(), task.id())
                .await?
            || !self.integration_gates(tx, &project, &task, &stored).await?
        {
            return Err(invalid(
                "requires a resumed current Task visit, open Project and current candidate gates",
            ));
        }
        if stored.intent.is_none()
            && tx
                .integration_ever_authorized_apply(stored.operation.id)
                .await?
        {
            return Err(invalid(
                "missing immutable intent after apply authorization requires investigation",
            ));
        }
        if resolution == "accept"
            && (stored.intent.is_none()
                || stored
                    .result
                    .as_ref()
                    .is_none_or(|result| result.code != IntegrationCode::Applied))
        {
            return Err(invalid(
                "accept requires a recorded Applied observation; a fresh reconciliation will verify it again",
            ));
        }
        stored.resolution = Some(resolution.into());
        stored.core_instance = self.instance_id;
        stored.expected_task_revision = expected;
        stored.command = None;
        tx.save_git_integration(&stored).await?;
        let previous = project.revision();
        let now = crate::canonical_clock::project_mutation_time(&project);
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::GitIntegrationChanged,
            self.actors.human,
            command_id,
            Some(reason.clone()),
            event_payload([("operation_id", json!(id)), ("action", json!(resolution))]),
            now,
        )?;
        crate::command::finish_command(
            tx,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(crate::command::resource("git_integration", id)),
        )
        .await
    }
}
