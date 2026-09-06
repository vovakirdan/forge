//! Core-owned audit and Task pause after fenced runtime observations.
use super::*;
pub(super) fn stale_observation_event(
    project: &Project,
    identity: &SupervisorIdentity,
    observation: &validation::ParsedObservation,
    expected_lease_fencing_token: u64,
    expected_environment_epoch: u64,
    now: Timestamp,
) -> Result<DomainEvent, CoreError> {
    Ok(event(
        project.id(),
        AggregateRef::Project(project.id()),
        project.revision(),
        DomainEventKind::RunObservationIgnored,
        identity.actor(),
        CommandId::from(observation.message_id),
        None,
        event_payload([
            ("reason_code", json!("stale_fence_or_environment_epoch")),
            ("run_id", json!(observation.run_id)),
            ("state", json!(observed_state_name(observation.state))),
            ("sequence", json!(observation.sequence)),
            (
                "lease_fencing_token",
                json!(observation.lease_fencing_token),
            ),
            ("environment_epoch", json!(observation.environment_epoch)),
            (
                "expected_lease_fencing_token",
                json!(expected_lease_fencing_token),
            ),
            (
                "expected_environment_epoch",
                json!(expected_environment_epoch),
            ),
        ]),
        now,
    )?)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fenced Run fact and Core audit identity must remain explicit at this canonical reconciliation boundary"
)]
pub(super) async fn pause_interrupted_task(
    transaction: &mut forge_storage::StorageTransaction<'_>,
    project: &mut Project,
    task_id: TaskId,
    run_id: uuid::Uuid,
    observed_state: RunObservedState,
    actor: forge_domain::Actor,
    command_id: CommandId,
    now: Timestamp,
) -> Result<Option<DomainEvent>, CoreError> {
    let stored = load_scoped_task(transaction, project, task_id).await?;
    let previous_task_revision = stored.task.revision().get();
    let mut task = stored.task;
    if task.lifecycle().is_terminal() {
        return Ok(None);
    }
    task.add_wait_condition(
        TaskWaitCondition::new(
            WaitConditionId::new(),
            TaskWaitKind::Interrupted,
            // Recovery addresses the wait by Run, independent of the last
            // physical observation (which remains in the audit event below).
            Some(format!("recovery_run={run_id}")),
            actor,
            now,
        )?,
        now,
    )?;
    let persistence = retained_persistence(project, &task, stored.persistence)?;
    persist_task_and_project(
        transaction,
        project,
        &task,
        persistence,
        previous_task_revision,
        now,
    )
    .await?;
    Ok(Some(task_event(
        project,
        &task,
        DomainEventKind::TaskWaiting,
        actor,
        command_id,
        now,
        event_payload([
            ("reason_code", json!("run_terminated_before_stage_outcome")),
            ("run_id", json!(run_id)),
            ("observed_state", json!(observed_state_name(observed_state))),
        ]),
    )?))
}

pub(in crate::supervisor) fn refused_fenced_write(result: FencedWrite) -> InboundResult {
    match result {
        FencedWrite::Applied => InboundResult::Accepted("Run fence accepted"),
        FencedWrite::Ignored => InboundResult::Ignored("stale_or_duplicate_run_message"),
        FencedWrite::SequenceGap { .. } => {
            InboundResult::Rejected("run_sequence_gap", "Run message sequence is not contiguous")
        }
        FencedWrite::InvalidObservedStateTransition { .. } => InboundResult::Rejected(
            "invalid_observed_state_transition",
            "Observed Run state would regress its canonical lifecycle",
        ),
        FencedWrite::Missing => {
            InboundResult::Rejected("run_not_found", "Run does not exist in canonical state")
        }
    }
}
