use super::*;
use forge_domain::{
    ArtifactId, HandoffArtifactAcceptance, HandoffArtifactReference, HandoffOutcome,
    HandoffProducer, OutcomeKey, TaskHandoff, TaskHandoffInput, Timestamp,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn persist(
    tx: &mut StorageTransaction<'_>,
    source: &HookCompletion,
    task: &Task,
    actor: forge_domain::Actor,
    outcome: OutcomeKey,
    artifact: ArtifactId,
    now: Timestamp,
) -> Result<(), CoreError> {
    let handoff = TaskHandoff::new(TaskHandoffInput {
        id: Uuid::now_v7(),
        project_id: source.project_id,
        task_id: source.task_id,
        actor,
        producer: HandoffProducer::SystemAction {
            operation_id: source.invocation_id,
        },
        outcome: HandoffOutcome::Completed { outcome },
        source_stage_id: source.stage_id.clone(),
        target_stage_id: task
            .current_stage_id()
            .cloned()
            .unwrap_or_else(|| source.stage_id.clone()),
        artifacts: vec![HandoffArtifactReference {
            artifact_id: artifact,
            acceptance: HandoffArtifactAcceptance::Submitted,
            acceptance_id: None,
        }],
        evidence: vec![],
        incident_id: None,
        work_surface_id: source.surface_id,
        last_observed_state: None,
        created_at: now,
    })?;
    tx.insert_hook_handoff(source.invocation_id, &handoff)
        .await?;
    Ok(())
}
