//! A manual stage produces its own handoff; earlier worker context is retained separately.
use super::{CommandError, CommandTransaction};
use forge_domain::{
    Actor, ArtifactId, CommandId, HandoffArtifactAcceptance, HandoffArtifactReference,
    HandoffOutcome, HandoffProducer, OutcomeKey, StageId, Task, TaskHandoff, TaskHandoffInput,
    TaskWorkSurface, Timestamp,
};
use std::collections::BTreeSet;
use uuid::Uuid;

#[expect(
    clippy::too_many_arguments,
    reason = "explicit accepted outcome and immutable command scope"
)]
pub(super) async fn persist(
    tx: &mut impl CommandTransaction,
    task: &Task,
    actor: Actor,
    command_id: CommandId,
    source_stage_id: StageId,
    outcome: OutcomeKey,
    artifacts: BTreeSet<ArtifactId>,
    now: Timestamp,
) -> Result<(), CommandError> {
    let target_stage_id = task
        .current_stage_id()
        .cloned()
        .unwrap_or_else(|| source_stage_id.clone());
    let handoff = TaskHandoff::new(TaskHandoffInput {
        id: Uuid::now_v7(),
        project_id: task.project_id(),
        task_id: task.id(),
        actor,
        producer: HandoffProducer::Command { command_id },
        outcome: HandoffOutcome::Completed { outcome },
        source_stage_id,
        target_stage_id,
        artifacts: artifacts
            .into_iter()
            .map(|artifact_id| HandoffArtifactReference {
                artifact_id,
                acceptance: HandoffArtifactAcceptance::Submitted,
                acceptance_id: None,
            })
            .collect(),
        evidence: vec![],
        incident_id: None,
        work_surface_id: match task.work_surface() {
            TaskWorkSurface::None => None,
            TaskWorkSurface::Git(binding) => Some(binding.surface_id),
        },
        last_observed_state: None,
        created_at: now,
    })?;
    tx.insert_command_handoff(&handoff).await?;
    Ok(())
}
