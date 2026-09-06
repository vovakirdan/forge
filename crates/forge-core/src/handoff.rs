//! Canonical handoff assembly at the result/interrupt transaction boundary.

use forge_domain::{
    Actor, HandoffArtifactAcceptance, HandoffArtifactReference, HandoffOutcome, HandoffProducer,
    OutcomeKey, StageId, Task, TaskHandoff, TaskHandoffInput, Timestamp,
};
use forge_storage::{RunProjection, StorageTransaction};
use uuid::Uuid;

use crate::CoreError;

/// A handoff carries canonical links, not a model-generated reconstruction.
pub(crate) async fn persist_handoff(
    transaction: &mut StorageTransaction<'_>,
    run: &RunProjection,
    task: &Task,
    actor: Actor,
    outcome: Option<OutcomeKey>,
    incident_id: Option<Uuid>,
    now: Timestamp,
) -> Result<(), CoreError> {
    let source = StageId::new(run.stage_id.clone())?;
    let accepted = outcome.is_some();
    let target = if accepted {
        task.current_stage_id()
            .cloned()
            .unwrap_or_else(|| source.clone())
    } else {
        source.clone()
    };
    let work_surface_id = run
        .run_spec
        .get("surface_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let handoff = TaskHandoff::new(TaskHandoffInput {
        id: Uuid::now_v7(),
        project_id: run.project_id,
        task_id: run.task_id,
        actor,
        producer: HandoffProducer::Run { run_id: run.id },
        outcome: outcome.map_or(HandoffOutcome::Interrupted, |outcome| {
            HandoffOutcome::Completed { outcome }
        }),
        source_stage_id: source,
        target_stage_id: target,
        artifacts: task
            .artifact_links()
            .iter()
            .map(|link| HandoffArtifactReference {
                artifact_id: link.artifact_id(),
                acceptance: HandoffArtifactAcceptance::Submitted,
                acceptance_id: None,
            })
            .collect(),
        evidence: vec![],
        incident_id,
        work_surface_id,
        last_observed_state: Some(format!("{:?}", run.observed_state).to_lowercase()),
        created_at: now,
    })?;
    let body = serde_json::to_value(handoff).map_err(|_| CoreError::InvalidTransport {
        field: "handoff",
        reason: "cannot serialize canonical handoff".into(),
    })?;
    transaction
        .insert_run_handoff(run.project_id, run.task_id, run.id, accepted, &body)
        .await?;
    Ok(())
}
