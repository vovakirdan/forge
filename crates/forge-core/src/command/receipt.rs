//! Runtime extensions use the same application finalization as M0 commands.

use forge_application::CommandEnvelope;
use forge_domain::{Actor, CommandId, DomainEvent, Project};
use forge_protocol::wire::{CommandReceipt, ResourceReference};
use forge_storage::StorageTransaction;

use crate::CoreError;

pub(crate) use forge_application::engine::receipt::resource;

#[expect(
    clippy::too_many_arguments,
    reason = "runtime extensions preserve the common audit boundary"
)]
pub(crate) async fn finish_command(
    transaction: &mut StorageTransaction<'_>,
    project: &Project,
    envelope: &CommandEnvelope,
    request_hash: &str,
    command_id: CommandId,
    actor: Actor,
    events: Vec<DomainEvent>,
    resource: Option<ResourceReference>,
) -> Result<CommandReceipt, CoreError> {
    forge_application::finish_command(
        transaction,
        project,
        envelope,
        request_hash,
        command_id,
        actor,
        events,
        resource,
    )
    .await
    .map_err(Into::into)
}
