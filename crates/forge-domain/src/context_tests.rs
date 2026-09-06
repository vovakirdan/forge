use uuid::Uuid;

use crate::{
    Actor, ActorId, ArtifactId, ContextSnapshot, ContextSnapshotInput, EmployeeId,
    HandoffArtifactAcceptance, HandoffArtifactReference, HandoffOutcome, HandoffProducer,
    PipelineVersionId, ProjectId, StageId, TaskHandoff, TaskHandoffInput, TaskId, TaskProperties,
    TaskScope, TaskSpec, TaskSpecInput, Timestamp,
};

fn snapshot_input() -> ContextSnapshotInput {
    ContextSnapshotInput {
        context_snapshot_id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        task_id: TaskId::new(),
        run_id: Uuid::now_v7(),
        employee_id: EmployeeId::new(),
        pipeline_version_id: PipelineVersionId::new(),
        stage_id: StageId::new("work").expect("stage"),
        task_revision_before_dispatch: 3,
        task_spec: TaskSpec::new(TaskSpecInput {
            title: "Preserve context".to_owned(),
            description: "Original intent".to_owned(),
            rationale: None,
            definition_of_done: Some("Verified evidence".to_owned()),
            scope: TaskScope::default(),
            properties: TaskProperties::default(),
            labels: Default::default(),
        })
        .expect("task spec"),
        system_policy_revision: "system/v1".to_owned(),
        employee_prompt_revision: "employee/v2".to_owned(),
        capability_grants: vec!["artifact.submit".to_owned()],
        tool_catalog_revision: "tools/v1".to_owned(),
        run_spec_id: Uuid::now_v7(),
        prior_handoff: None,
        artifacts: Vec::new(),
        control_instruction: None,
        created_at: Timestamp::now_utc(),
    }
}

fn handoff_input() -> TaskHandoffInput {
    TaskHandoffInput {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        task_id: TaskId::new(),
        actor: Actor::core(ActorId::new()),
        producer: HandoffProducer::Run {
            run_id: Uuid::now_v7(),
        },
        outcome: HandoffOutcome::Interrupted,
        source_stage_id: StageId::new("work").expect("stage"),
        target_stage_id: StageId::new("work").expect("stage"),
        artifacts: Vec::new(),
        evidence: Vec::new(),
        incident_id: Some(Uuid::now_v7()),
        work_surface_id: Some(Uuid::now_v7()),
        last_observed_state: Some("stopped".to_owned()),
        created_at: Timestamp::now_utc(),
    }
}

#[test]
fn issued_context_is_detached_from_subsequent_input_changes() {
    let mut input = snapshot_input();
    let snapshot = ContextSnapshot::new(input.clone()).expect("snapshot");
    let frozen = serde_json::to_value(&snapshot).expect("serialize snapshot");
    input.employee_prompt_revision = "employee/v3".to_owned();
    input.task_revision_before_dispatch += 1;
    input.capability_grants.clear();
    assert_eq!(
        serde_json::to_value(&snapshot).expect("serialize snapshot"),
        frozen
    );
    assert_eq!(
        serde_json::from_value::<ContextSnapshot>(frozen).expect("restore"),
        snapshot
    );
}

#[test]
fn interrupted_handoff_cannot_transition_to_review_even_after_deserialization() {
    let input = handoff_input();
    let handoff = TaskHandoff::new(input).expect("interrupted handoff");
    let mut serialized = serde_json::to_value(&handoff).expect("serialized");
    serialized["target_stage_id"] = serde_json::json!("review");
    assert!(serde_json::from_value::<TaskHandoff>(serialized).is_err());
    assert_eq!(
        handoff.data().source_stage_id,
        handoff.data().target_stage_id
    );
}

#[test]
fn interrupted_handoff_requires_a_canonical_incident() {
    let mut input = handoff_input();
    input.incident_id = None;
    assert!(TaskHandoff::new(input).is_err());
}

#[test]
fn handoff_cannot_claim_acceptance_without_acceptance_record() {
    let mut input = handoff_input();
    input.artifacts.push(HandoffArtifactReference {
        artifact_id: ArtifactId::new(),
        acceptance: HandoffArtifactAcceptance::Accepted,
        acceptance_id: None,
    });
    assert!(TaskHandoff::new(input).is_err());
}

#[test]
fn context_rejects_handoff_from_another_task() {
    let mut input = snapshot_input();
    input.prior_handoff = Some(TaskHandoff::new(handoff_input()).expect("handoff"));
    assert!(ContextSnapshot::new(input).is_err());
}

#[test]
fn first_stage_context_needs_no_synthetic_handoff() {
    assert!(
        ContextSnapshot::new(snapshot_input())
            .expect("initial context")
            .data()
            .prior_handoff
            .is_none()
    );
}
