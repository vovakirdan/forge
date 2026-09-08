use std::collections::BTreeSet;

use super::{
    ArtifactRequirement, ArtifactRequirementScope, ExecutorKind, OutcomeKey, PipelineStage,
    PipelineTransition, PipelineTransitionTarget, PipelineVersion, PipelineVersionInput,
    StageOutcomeSubmission,
};
use crate::{
    Actor, ActorId, Artifact, ArtifactBody, ArtifactId, ArtifactKind, ArtifactProducer,
    CancellationReason, CancellationReasonCatalog, LifecycleStatus, NewArtifact, NewTask,
    PipelineId, PipelineVersionId, PriorityScheme, ProjectId, ProjectLocalSequence, StageId, Task,
    TaskId, TaskKey, TaskKind, TaskPipelineBinding, TaskProperties, TaskPropertySchema, TaskScope,
    TaskSource, TaskSpec, TaskSpecInput, TaskWaitCondition, TaskWaitKind, Timestamp,
    WaitConditionId,
};

mod artifact_history;
mod catalog_management;

fn stage(
    id: &str,
    executor_kind: ExecutorKind,
    transitions: Vec<PipelineTransition>,
) -> PipelineStage {
    PipelineStage::new(
        StageId::new(id).expect("stage id"),
        id,
        executor_kind,
        transitions,
    )
    .expect("stage")
}

fn transition(outcome: &str, target: PipelineTransitionTarget) -> PipelineTransition {
    PipelineTransition::new(OutcomeKey::new(outcome).expect("outcome"), target, vec![])
        .expect("transition")
}

fn transition_with_requirements(
    outcome: &str,
    target: PipelineTransitionTarget,
    requirements: Vec<ArtifactRequirement>,
) -> PipelineTransition {
    PipelineTransition::new(
        OutcomeKey::new(outcome).expect("outcome"),
        target,
        requirements,
    )
    .expect("transition")
}

fn new_task(version: &PipelineVersion, kind: TaskKind, entry_stage: &str) -> Task {
    let priority_scheme = PriorityScheme::default_three_levels().expect("priority scheme");
    let created_at = Timestamp::now_utc();
    let spec = TaskSpec::new(TaskSpecInput {
        title: "Bounded pipeline test".to_owned(),
        description: String::new(),
        rationale: None,
        definition_of_done: Some("Artifacts are linked".to_owned()),
        scope: TaskScope::default(),
        properties: TaskProperties::empty(),
        labels: BTreeSet::new(),
    })
    .expect("task spec");
    let mut task = Task::new_draft(
        NewTask {
            id: TaskId::new(),
            project_id: version.project_id(),
            key: TaskKey::new(ProjectLocalSequence::new(1).expect("sequence")),
            kind,
            spec,
            priority_level_id: priority_scheme.default_level_id().clone(),
            pipeline: TaskPipelineBinding::new(
                version.pipeline_id(),
                version.id(),
                StageId::new(entry_stage).expect("entry stage"),
            ),
            source: TaskSource::Human,
            created_by: Actor::human(ActorId::new()),
            created_at,
        },
        &priority_scheme,
    )
    .expect("draft task");
    task.approve(
        &TaskPropertySchema::new([]).expect("property schema"),
        created_at,
    )
    .expect("approve task");
    task
}

fn artifact(task: &Task, kind: &str, title: &str) -> Artifact {
    Artifact::new(
        ArtifactId::new(),
        NewArtifact {
            project_id: task.project_id(),
            kind: ArtifactKind::new(kind).expect("artifact kind"),
            title: title.to_owned(),
            body: ArtifactBody::inline_json(serde_json::json!({"evidence": title})),
            metadata: serde_json::json!({}),
            created_by: Actor::human(ActorId::new()),
            created_at: Timestamp::now_utc(),
        },
    )
    .expect("artifact")
}

fn cancellation_catalog() -> CancellationReasonCatalog {
    CancellationReasonCatalog::new([CancellationReason::unspecified().expect("reason")])
        .expect("catalog")
}

#[test]
fn pipeline_rejects_unreachable_and_terminal_free_graphs() {
    let result = PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id: PipelineId::new(),
        project_id: ProjectId::new(),
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: StageId::new("work").expect("entry"),
        max_stage_visits: None,
        stages: vec![
            stage(
                "work",
                ExecutorKind::Employee,
                vec![transition(
                    "again",
                    PipelineTransitionTarget::Stage(StageId::new("work").expect("stage")),
                )],
            ),
            stage(
                "orphan",
                ExecutorKind::Employee,
                vec![transition("done", PipelineTransitionTarget::Done)],
            ),
        ],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    });
    assert!(result.is_err());
}

#[test]
fn pipeline_rejects_an_unbounded_retry_cycle_even_when_a_terminal_route_exists() {
    let result = PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id: PipelineId::new(),
        project_id: ProjectId::new(),
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: StageId::new("work").expect("entry"),
        max_stage_visits: None,
        stages: vec![stage(
            "work",
            ExecutorKind::Employee,
            vec![
                transition(
                    "retry",
                    PipelineTransitionTarget::Stage(StageId::new("work").expect("stage")),
                ),
                transition("done", PipelineTransitionTarget::Done),
            ],
        )],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    });

    assert!(result.is_err());
}

#[test]
fn bounded_retry_cycle_refuses_another_stage_entry_after_its_budget() {
    let project_id = ProjectId::new();
    let pipeline_id = PipelineId::new();
    let version = PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id,
        project_id,
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: StageId::new("work").expect("entry"),
        max_stage_visits: Some(2),
        stages: vec![stage(
            "work",
            ExecutorKind::Employee,
            vec![
                transition(
                    "retry",
                    PipelineTransitionTarget::Stage(StageId::new("work").expect("stage")),
                ),
                transition("done", PipelineTransitionTarget::Done),
            ],
        )],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    })
    .expect("bounded retry graph");
    let mut task = new_task(&version, TaskKind::Delivery, "work");
    version
        .enter_entry_stage(&mut task, None, Timestamp::now_utc())
        .expect("enter first stage");
    version
        .start_entry_employee_stage(&mut task, Timestamp::now_utc())
        .expect("start first employee stage");
    version
        .apply_outcome(
            &mut task,
            StageOutcomeSubmission {
                outcome: OutcomeKey::new("retry").expect("outcome"),
                outcome_artifact_ids: BTreeSet::new(),
                submitted_by: Actor::employee(ActorId::new()),
                submitted_at: Timestamp::now_utc(),
                cancellation_reason_id: None,
                cancellation_note: None,
            },
            &cancellation_catalog(),
            None,
            None,
        )
        .expect("enter the final allowed visit");
    let revision = task.revision();

    let result = version.apply_outcome(
        &mut task,
        StageOutcomeSubmission {
            outcome: OutcomeKey::new("retry").expect("outcome"),
            outcome_artifact_ids: BTreeSet::new(),
            submitted_by: Actor::employee(ActorId::new()),
            submitted_at: Timestamp::now_utc(),
            cancellation_reason_id: None,
            cancellation_note: None,
        },
        &cancellation_catalog(),
        None,
        None,
    );

    assert!(matches!(
        result,
        Err(crate::DomainError::StageVisitLimitExceeded { maximum: 2 })
    ));
    assert_eq!(task.revision(), revision);
}

#[test]
fn pipeline_exposes_required_artifact_contract() {
    let requirement = ArtifactRequirement::new(
        ArtifactKind::new(ArtifactKind::CHANGE_SET).expect("kind"),
        1,
    )
    .expect("requirement");
    assert_eq!(requirement.minimum_count(), 1);
}

#[test]
fn manual_stage_outcome_cannot_bypass_an_additional_active_wait() {
    let project_id = ProjectId::new();
    let pipeline_id = PipelineId::new();
    let version = PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id,
        project_id,
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: StageId::new("decision").expect("entry stage"),
        max_stage_visits: None,
        stages: vec![stage(
            "decision",
            ExecutorKind::Human,
            vec![transition("accept", PipelineTransitionTarget::Done)],
        )],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    })
    .expect("pipeline version");
    let mut task = new_task(&version, TaskKind::Delivery, "decision");
    let stage_wait = TaskWaitCondition::for_stage(
        WaitConditionId::new(),
        StageId::new("decision").expect("stage"),
        task.current_stage_visit().expect("entry visit"),
        None,
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    )
    .expect("stage wait");
    let stage_wait_id = stage_wait.id();
    version
        .enter_entry_stage(&mut task, Some(stage_wait), Timestamp::now_utc())
        .expect("enter human stage");
    task.add_wait_condition(
        TaskWaitCondition::new(
            WaitConditionId::new(),
            TaskWaitKind::ManualPause,
            Some("Manager requested a pause".to_owned()),
            Actor::human(ActorId::new()),
            Timestamp::now_utc(),
        )
        .expect("pause wait"),
        Timestamp::now_utc(),
    )
    .expect("add generic wait");
    let revision_before = task.revision();

    let result = version.apply_outcome(
        &mut task,
        StageOutcomeSubmission {
            outcome: OutcomeKey::new("accept").expect("outcome"),
            outcome_artifact_ids: BTreeSet::new(),
            submitted_by: Actor::human(ActorId::new()),
            submitted_at: Timestamp::now_utc(),
            cancellation_reason_id: None,
            cancellation_note: None,
        },
        &cancellation_catalog(),
        Some(stage_wait_id),
        None,
    );

    assert!(result.is_err());
    assert_eq!(task.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(task.revision(), revision_before);
    assert_eq!(task.wait_conditions().count(), 2);
}

#[test]
fn current_stage_requirement_rejects_evidence_from_a_previous_visit() {
    let project_id = ProjectId::new();
    let pipeline_id = PipelineId::new();
    let version = PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id,
        project_id,
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: StageId::new("review").expect("entry stage"),
        max_stage_visits: Some(3),
        stages: vec![
            stage(
                "review",
                ExecutorKind::Employee,
                vec![
                    transition(
                        "rework",
                        PipelineTransitionTarget::Stage(StageId::new("work").expect("stage")),
                    ),
                    transition_with_requirements(
                        "accept",
                        PipelineTransitionTarget::Done,
                        vec![
                            ArtifactRequirement::new(
                                ArtifactKind::new(ArtifactKind::REVIEW_RESULT).expect("kind"),
                                1,
                            )
                            .expect("requirement"),
                        ],
                    ),
                ],
            ),
            stage(
                "work",
                ExecutorKind::Employee,
                vec![transition(
                    "ready_for_review",
                    PipelineTransitionTarget::Stage(StageId::new("review").expect("stage")),
                )],
            ),
        ],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    })
    .expect("pipeline version");
    let mut task = new_task(&version, TaskKind::Delivery, "review");
    version
        .enter_entry_stage(&mut task, None, Timestamp::now_utc())
        .expect("enter first review");
    version
        .start_entry_employee_stage(&mut task, Timestamp::now_utc())
        .expect("start first review");
    let first_review = artifact(&task, ArtifactKind::REVIEW_RESULT, "First review");
    task.attach_artifact(
        &first_review,
        ArtifactProducer::EmployeeRun,
        Actor::employee(ActorId::new()),
        None,
        Timestamp::now_utc(),
    )
    .expect("attach first review");
    version
        .apply_outcome(
            &mut task,
            StageOutcomeSubmission {
                outcome: OutcomeKey::new("rework").expect("outcome"),
                outcome_artifact_ids: BTreeSet::new(),
                submitted_by: Actor::employee(ActorId::new()),
                submitted_at: Timestamp::now_utc(),
                cancellation_reason_id: None,
                cancellation_note: None,
            },
            &cancellation_catalog(),
            None,
            None,
        )
        .expect("enter work");
    version
        .apply_outcome(
            &mut task,
            StageOutcomeSubmission {
                outcome: OutcomeKey::new("ready_for_review").expect("outcome"),
                outcome_artifact_ids: BTreeSet::new(),
                submitted_by: Actor::employee(ActorId::new()),
                submitted_at: Timestamp::now_utc(),
                cancellation_reason_id: None,
                cancellation_note: None,
            },
            &cancellation_catalog(),
            None,
            None,
        )
        .expect("enter second review");
    let revision_before = task.revision();

    let result = version.apply_outcome(
        &mut task,
        StageOutcomeSubmission {
            outcome: OutcomeKey::new("accept").expect("outcome"),
            outcome_artifact_ids: BTreeSet::from([first_review.id()]),
            submitted_by: Actor::employee(ActorId::new()),
            submitted_at: Timestamp::now_utc(),
            cancellation_reason_id: None,
            cancellation_note: None,
        },
        &cancellation_catalog(),
        None,
        None,
    );

    assert!(result.is_err());
    assert_eq!(task.lifecycle(), LifecycleStatus::InProgress);
    assert_eq!(task.revision(), revision_before);
}
