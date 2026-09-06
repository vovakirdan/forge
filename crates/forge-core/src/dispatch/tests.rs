use forge_domain::{
    Actor, ActorId, ArtifactKind, ArtifactRequirement, ArtifactRequirementScope, ExecutorKind,
    NewTask, OutcomeKey, PipelineId, PipelineStage, PipelineTransition, PipelineTransitionTarget,
    PipelineVersionId, PriorityScheme, ProjectId, ProjectLocalSequence, StageId, Task, TaskId,
    TaskKey, TaskKind, TaskPipelineBinding, TaskProperties, TaskPropertySchema, TaskScope,
    TaskSource, TaskSpec, TaskSpecInput, Timestamp,
};
use serde_json::json;

use super::fake_run_spec;

fn task_with_history_artifact(kind: &str) -> Task {
    let priority_scheme = PriorityScheme::default_three_levels().expect("priority scheme");
    let now = Timestamp::now_utc();
    let mut task = Task::new_draft(
        NewTask {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            key: TaskKey::new(ProjectLocalSequence::new(1).expect("sequence")),
            kind: TaskKind::Delivery,
            spec: TaskSpec::new(TaskSpecInput {
                title: "M0 fake test".to_owned(),
                description: String::new(),
                rationale: None,
                definition_of_done: Some("Historical evidence is available".to_owned()),
                scope: TaskScope::default(),
                properties: TaskProperties::empty(),
                labels: Default::default(),
            })
            .expect("task spec"),
            priority_level_id: priority_scheme.default_level_id().clone(),
            pipeline: TaskPipelineBinding::new(
                PipelineId::new(),
                PipelineVersionId::new(),
                StageId::new("work").expect("stage"),
            ),
            source: TaskSource::Human,
            created_by: Actor::human(ActorId::new()),
            created_at: now,
        },
        &priority_scheme,
    )
    .expect("draft task");
    task.approve(&TaskPropertySchema::new([]).expect("schema"), now)
        .expect("approve task");
    let artifact = forge_domain::Artifact::new(
        forge_domain::ArtifactId::new(),
        forge_domain::NewArtifact {
            project_id: task.project_id(),
            kind: ArtifactKind::new(kind).expect("artifact kind"),
            title: "Historical evidence".to_owned(),
            body: forge_domain::ArtifactBody::inline_json(json!({})),
            metadata: json!({}),
            created_by: Actor::human(ActorId::new()),
            created_at: now,
        },
    )
    .expect("artifact");
    task.attach_artifact(
        &artifact,
        forge_domain::ArtifactProducer::Human,
        Actor::human(ActorId::new()),
        None,
        now,
    )
    .expect("attach artifact");
    task
}

#[test]
fn fake_spec_supplies_each_current_stage_requirement() {
    let stage = PipelineStage::new(
        StageId::new("work").expect("stable stage key"),
        "Work",
        ExecutorKind::Employee,
        [PipelineTransition::new(
            OutcomeKey::new("implemented").expect("stable outcome key"),
            PipelineTransitionTarget::Done,
            vec![
                ArtifactRequirement::new(
                    ArtifactKind::new("change_set").expect("stable artifact key"),
                    2,
                )
                .expect("positive requirement"),
            ],
        )
        .expect("valid transition")],
    )
    .expect("valid stage");

    let task = task_with_history_artifact("unrelated");
    let spec = fake_run_spec(&stage, &task).expect("compatible M0 stage");

    assert_eq!(spec["artifacts"].as_array().map(Vec::len), Some(2));
    assert_eq!(spec["stage_outcome"]["outcome"], "implemented");
}

#[test]
fn fake_spec_cites_satisfied_task_history_evidence() {
    let requirement = ArtifactRequirement::new_with_scope(
        ArtifactKind::new("change_set").expect("artifact key"),
        1,
        ArtifactRequirementScope::TaskHistory,
    )
    .expect("positive requirement");
    let stage = PipelineStage::new(
        StageId::new("review").expect("stable stage key"),
        "Review",
        ExecutorKind::Employee,
        [PipelineTransition::new(
            OutcomeKey::new("approved").expect("stable outcome key"),
            PipelineTransitionTarget::Done,
            vec![requirement],
        )
        .expect("valid transition")],
    )
    .expect("valid stage");
    let task = task_with_history_artifact("change_set");

    let spec = fake_run_spec(&stage, &task).expect("history evidence is available");

    assert!(spec["artifacts"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        spec["stage_outcome"]["artifact_ids"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}
