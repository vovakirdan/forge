use super::*;
use forge_domain::{
    ArtifactId, HandoffArtifactAcceptance, HandoffArtifactReference, HandoffOutcome,
    HandoffProducer, OutcomeKey, TaskHandoff, TaskHandoffInput,
};

pub(super) async fn persist_system_handoff(
    tx: &mut StorageTransaction<'_>,
    stored: &StoredIntegration,
    task: &Task,
    actor: forge_domain::Actor,
    outcome: OutcomeKey,
    artifact: ArtifactId,
    now: Timestamp,
) -> Result<(), CoreError> {
    let op = &stored.operation;
    let handoff = TaskHandoff::new(TaskHandoffInput {
        id: Uuid::now_v7(),
        project_id: op.project_id,
        task_id: op.task_id,
        actor,
        producer: HandoffProducer::SystemAction {
            operation_id: op.id,
        },
        outcome: HandoffOutcome::Completed { outcome },
        source_stage_id: op.stage_id.clone(),
        target_stage_id: task
            .current_stage_id()
            .cloned()
            .unwrap_or_else(|| op.stage_id.clone()),
        artifacts: vec![HandoffArtifactReference {
            artifact_id: artifact,
            acceptance: HandoffArtifactAcceptance::Submitted,
            acceptance_id: None,
        }],
        evidence: vec![],
        incident_id: None,
        work_surface_id: Some(op.binding.surface_id),
        last_observed_state: None,
        created_at: now,
    })?;
    tx.insert_integration_handoff(op.id, &handoff).await?;
    Ok(())
}
