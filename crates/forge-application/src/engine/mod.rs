//! One command semantics path, borrowing an atomic repository owned by its caller.
mod commands;
mod communication;
mod context;
pub mod dependency_waits;
mod employee_commands;
mod error;
pub mod event;
mod external_handoff;
pub mod external_outcome;
mod finding;
mod manager_commands;
mod manager_constraints;
mod manager_resume;
pub mod pipeline_access;
mod pipeline_commands;
pub mod ports;
pub mod receipt;
pub mod records;
mod repository_commands;
pub mod resolver_acceptance;
mod resolver_answers;
mod resolver_commands;
mod resolver_raise;
pub mod run_control;
pub mod scheduler;
mod task_commands;
pub mod task_control;
mod task_creation;
pub mod task_support;
#[cfg(test)]
mod tests;

use crate::CommandEnvelope;
pub use context::{Clock, CommandContext, SystemClock};
pub use error::{CommandError, RepositoryError};
use forge_domain::{Actor, CommandId, Project, ProjectId, Timestamp};
use forge_protocol::wire::CommandReceipt;
pub use ports::{CommandTransaction, *};
pub use receipt::{command_fingerprint, command_name, finish_command, resource};

/// A sealed result of authorization and Project locking in the caller's transaction.
/// Consume it in the same transaction; it does not own commit or rollback.
pub struct PreparedCommand {
    state: PreparedCommandState,
    project_id: ProjectId,
    actor: Actor,
    core_actor: Actor,
    request_hash: String,
    idempotency_key: String,
}

/// Read-only preparation view; only a sealed PreparedCommand can enter the M0 executor.
pub enum PreparedCommandState {
    /// A matching prior receipt, resolved before the optimistic revision check.
    Replay(CommandReceipt),
    /// Authorized fresh command values protected by the caller's still-open transaction.
    Ready {
        project: Option<Project>,
        request_hash: String,
        command_id: CommandId,
        now: Timestamp,
    },
}

impl PreparedCommand {
    /// Inspects whether Core must handle an M1 runtime extension or a replay.
    #[must_use]
    pub fn state(&self) -> &PreparedCommandState {
        &self.state
    }

    /// Checks provenance before giving the trusted M1 boundary its prepared values.
    pub fn into_state(
        self,
        context: &CommandContext,
        envelope: &CommandEnvelope,
    ) -> Result<PreparedCommandState, CommandError> {
        self.validate(context, envelope)?;
        Ok(self.state)
    }

    fn validate(
        &self,
        context: &CommandContext,
        envelope: &CommandEnvelope,
    ) -> Result<(), CommandError> {
        context.authorize(envelope)?;
        if self.project_id != envelope.project_id
            || self.actor != context.actor
            || self.core_actor != context.core_actor
            || self.idempotency_key != envelope.idempotency_key.as_str()
            || self.request_hash != command_fingerprint(envelope, context.actor)?
        {
            return Err(CommandError::Forbidden);
        }
        Ok(())
    }
}

/// Canonical mutations retain the durable Project floor; operational clocks stay raw.
#[must_use]
pub fn project_mutation_time_at(project: &Project, observed_wall: Timestamp) -> Timestamp {
    observed_wall.max(project.updated_at())
}

/// Authorizes before replay or writes; locks before consulting operational time.
pub async fn prepare_command(
    transaction: &mut impl CommandTransaction,
    context: &CommandContext,
    envelope: &CommandEnvelope,
    clock: &dyn Clock,
) -> Result<PreparedCommand, CommandError> {
    context.authorize(envelope)?;
    let request_hash = command_fingerprint(envelope, context.actor)?;
    let command_id = CommandId::new();
    transaction
        .lock_project_creation(envelope.project_id)
        .await?;
    let project = transaction.lock_project(envelope.project_id).await?;
    let wall = clock.now();
    let now = project
        .as_ref()
        .map_or(wall, |project| project_mutation_time_at(project, wall));
    let seal = |state| PreparedCommand {
        state,
        project_id: envelope.project_id,
        actor: context.actor,
        core_actor: context.core_actor,
        request_hash: request_hash.clone(),
        idempotency_key: envelope.idempotency_key.as_str().to_owned(),
    };
    if let Some(project) = project.as_ref() {
        if let Some(receipt) =
            receipt::replay_if_present(transaction, envelope, &request_hash).await?
        {
            return Ok(seal(PreparedCommandState::Replay(receipt)));
        }
        project.require_revision(envelope.expected_project_revision)?;
    }
    Ok(seal(PreparedCommandState::Ready {
        project,
        request_hash: request_hash.clone(),
        command_id,
        now,
    }))
}

/// Applies only M0 commands. Commit, rollback, runtime delivery and dispatch stay with the owner.
pub async fn execute_in_transaction(
    transaction: &mut impl CommandTransaction,
    context: &CommandContext,
    envelope: &CommandEnvelope,
    clock: &dyn Clock,
) -> Result<CommandReceipt, CommandError> {
    require_m0(envelope)?;
    let prepared = prepare_command(transaction, context, envelope, clock).await?;
    execute_prepared_in_transaction(transaction, context, envelope, prepared, clock).await
}

/// Applies a sealed M0 preparation in its original transaction after rechecking authority.
/// Rejects M1 even when a matching runtime-extension receipt already exists.
pub async fn execute_prepared_in_transaction(
    transaction: &mut impl CommandTransaction,
    context: &CommandContext,
    envelope: &CommandEnvelope,
    prepared: PreparedCommand,
    clock: &dyn Clock,
) -> Result<CommandReceipt, CommandError> {
    require_m0(envelope)?;
    let engine = Engine::new(context, clock);
    match prepared.into_state(context, envelope)? {
        PreparedCommandState::Replay(receipt) => Ok(receipt),
        PreparedCommandState::Ready {
            project,
            request_hash,
            command_id,
            now,
        } => match project {
            Some(project) => {
                engine
                    .apply_existing_command(
                        transaction,
                        project,
                        envelope,
                        &request_hash,
                        command_id,
                        now,
                    )
                    .await
            }
            None => {
                engine
                    .create_project_command(transaction, envelope, &request_hash, command_id, now)
                    .await
            }
        },
    }
}

fn require_m0(envelope: &CommandEnvelope) -> Result<(), CommandError> {
    use forge_protocol::wire::CommandName;
    if matches!(
        envelope.name,
        CommandName::ConfigureBootRecoveryPolicy
            | CommandName::AcceptRunRecoveryAssessment
            | CommandName::EnrollCredential
            | CommandName::ConfigureEmployeeRuntime
            | CommandName::ConfigureProjectHook
            | CommandName::ImportTaskFileSnapshot
            | CommandName::CaptureTaskFileSnapshot
            | CommandName::AttachTaskFileInput
    ) {
        return Err(CommandError::UnsupportedCommand);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct EngineActors {
    human: Actor,
    core: Actor,
}
/// Shared semantic helpers for trusted Core orchestration and the command dispatcher.
/// Direct helper callers own authorization, Project locking, and rollback on any error.
pub struct Engine<'a> {
    actors: EngineActors,
    clock: &'a dyn Clock,
}
impl<'a> Engine<'a> {
    /// Binds audit identities and the operational clock; performs no repository work.
    #[must_use]
    pub fn new(context: &CommandContext, clock: &'a dyn Clock) -> Self {
        Self {
            actors: EngineActors {
                human: context.actor,
                core: context.core_actor,
            },
            clock,
        }
    }
}
