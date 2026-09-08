//! Registry mutation uses the ordinary prepared-command authorization and receipt.
use crate::{
    CoreError, CoreService,
    command::{finish_command, resource},
    event::{event, event_payload},
};
use forge_application::{CommandEnvelope, CommandPayload};
use forge_domain::{AggregateRef, CommandId, DomainEventKind, Project, Timestamp};
use forge_protocol::wire::CommandReceipt;
use forge_storage::StorageTransaction;
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn configure_project_hook(
        &self,
        tx: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        let CommandPayload::ConfigureProjectHook(input) = &envelope.payload else {
            return Err(CoreError::InvalidTransport {
                field: "command",
                reason: "not a project hook configuration".into(),
            });
        };
        let version = input.materialize(Uuid::now_v7(), project.id(), now)?;
        tx.insert_project_hook_version(&version).await?;
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::ProjectHookConfigured,
            self.actors.human,
            command_id,
            None,
            event_payload([
                ("hook_version_id", json!(version.id)),
                ("name", json!(version.name)),
                ("required", json!(version.required)),
            ]),
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
            Some(resource("project_hook_version", version.id)),
        )
        .await
    }
}
