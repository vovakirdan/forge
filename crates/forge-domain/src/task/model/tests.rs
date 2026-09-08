use serde_json::json;
mod git_binding;

use super::{
    CancellationRequest, CompletionData, CompletionSource, NewTask, Task, TaskKind,
    TaskPipelineBinding, TaskScope, TaskSource, TaskSpec, TaskSpecInput,
};
use crate::{
    Actor, ActorId, Artifact, ArtifactBody, ArtifactId, ArtifactKind, ArtifactProducer,
    CancellationReason, CancellationReasonCatalog, NewArtifact, PipelineId, PipelineVersionId,
    PriorityScheme, ProjectId, StageId, TaskId, TaskKey, TaskProperties, TaskPropertySchema,
    TaskWaitCondition, TaskWaitKind, Timestamp, WaitConditionId,
};

fn draft(kind: TaskKind) -> Task {
    let priority_scheme = PriorityScheme::default_three_levels().expect("valid priority scheme");
    let sequence = crate::ProjectLocalSequence::new(1).expect("positive sequence");
    let stage = StageId::new("work").expect("valid stage");
    let spec = TaskSpec::new(TaskSpecInput {
        title: "Do bounded work".to_owned(),
        description: String::new(),
        rationale: None,
        definition_of_done: Some("Evidence is attached".to_owned()),
        scope: TaskScope::default(),
        properties: TaskProperties::empty(),
        labels: Default::default(),
    })
    .expect("valid task spec");

    Task::new_draft(
        NewTask {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            key: TaskKey::new(sequence),
            kind,
            spec,
            priority_level_id: priority_scheme.default_level_id().clone(),
            pipeline: TaskPipelineBinding::new(PipelineId::new(), PipelineVersionId::new(), stage),
            source: TaskSource::Human,
            created_by: Actor::human(ActorId::new()),
            created_at: Timestamp::now_utc(),
        },
        &priority_scheme,
    )
    .expect("valid draft")
}

fn approve_and_start(task: &mut Task) {
    let schema = TaskPropertySchema::new([]).expect("empty schema");
    task.approve(&schema, Timestamp::now_utc())
        .expect("approve task");
    task.start(Timestamp::now_utc()).expect("start task");
}

#[test]
fn lifecycle_rejects_completion_before_the_first_stage_started() {
    let mut task = draft(TaskKind::Delivery);
    let schema = TaskPropertySchema::new([]).expect("empty schema");
    task.approve(&schema, Timestamp::now_utc())
        .expect("approve task");

    let result = task.complete(CompletionData::new(
        CompletionSource::StageOutcome,
        [],
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    ));

    assert!(result.is_err());
}

#[test]
fn run_attempt_requires_an_active_stage_and_advances_task_revision() {
    let mut task = draft(TaskKind::Delivery);
    let before_start = task.record_run_attempt(Timestamp::now_utc());
    assert!(before_start.is_err());

    approve_and_start(&mut task);
    let revision = task.revision();
    task.record_run_attempt(Timestamp::now_utc())
        .expect("active stage can receive a run attempt");

    assert!(task.revision() > revision);
}

#[test]
fn cancellation_without_a_reason_is_rejected() {
    let mut task = draft(TaskKind::Delivery);
    let catalog = CancellationReasonCatalog::new([
        CancellationReason::unspecified().expect("mandatory reason")
    ])
    .expect("valid catalog");

    let result = task.cancel(
        &catalog,
        CancellationRequest {
            reason_id: None,
            note: None,
            cancelled_by: Actor::human(ActorId::new()),
            cancelled_at: Timestamp::now_utc(),
        },
    );

    assert!(result.is_err());
}

#[test]
fn analysis_completion_requires_an_attached_analysis_result() {
    let mut task = draft(TaskKind::Analysis);
    approve_and_start(&mut task);

    let result = task.complete(CompletionData::new(
        CompletionSource::StageOutcome,
        [],
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    ));

    assert!(result.is_err());
}

#[test]
fn analysis_completion_accepts_linked_analysis_result() {
    let mut task = draft(TaskKind::Analysis);
    approve_and_start(&mut task);
    let artifact = Artifact::new(
        ArtifactId::new(),
        NewArtifact {
            project_id: task.project_id(),
            kind: ArtifactKind::new(ArtifactKind::ANALYSIS_RESULT).expect("valid kind"),
            title: "Investigation result".to_owned(),
            body: ArtifactBody::inline_json(json!({"verdict": "confirmed"})),
            metadata: json!({}),
            created_by: Actor::human(ActorId::new()),
            created_at: Timestamp::now_utc(),
        },
    )
    .expect("valid artifact");
    task.attach_artifact(
        &artifact,
        ArtifactProducer::Human,
        Actor::human(ActorId::new()),
        None,
        Timestamp::now_utc(),
    )
    .expect("attach artifact");

    task.complete(CompletionData::new(
        CompletionSource::StageOutcome,
        [artifact.id()],
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    ))
    .expect("complete analysis task");

    assert_eq!(task.lifecycle(), crate::LifecycleStatus::Done);
}

#[test]
fn waiting_task_resumes_only_after_the_last_condition_resolves() {
    let mut task = draft(TaskKind::Delivery);
    let schema = TaskPropertySchema::new([]).expect("empty schema");
    task.approve(&schema, Timestamp::now_utc())
        .expect("approve task");
    let first = TaskWaitCondition::new(
        WaitConditionId::new(),
        TaskWaitKind::Dependency,
        None,
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    )
    .expect("valid condition");
    let second = TaskWaitCondition::new(
        WaitConditionId::new(),
        TaskWaitKind::ManualPause,
        None,
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    )
    .expect("valid condition");
    let first_id = first.id();
    let second_id = second.id();
    task.add_wait_condition(first, Timestamp::now_utc())
        .expect("first wait");
    task.add_wait_condition(second, Timestamp::now_utc())
        .expect("second wait");

    let first_resolution = task
        .resolve_wait_condition(first_id, Timestamp::now_utc())
        .expect("resolve first wait");
    let second_resolution = task
        .resolve_wait_condition(second_id, Timestamp::now_utc())
        .expect("resolve second wait");

    assert!(!first_resolution);
    assert!(second_resolution);
    assert_eq!(task.lifecycle(), crate::LifecycleStatus::Ready);
}

#[test]
fn retry_exhausted_wait_cannot_be_resumed_as_a_normal_pause() {
    let mut task = draft(TaskKind::Delivery);
    let schema = TaskPropertySchema::new([]).expect("empty schema");
    task.approve(&schema, Timestamp::now_utc())
        .expect("approve task");
    task.start(Timestamp::now_utc()).expect("start task");
    let exhausted = TaskWaitCondition::new(
        WaitConditionId::new(),
        TaskWaitKind::RetryExhausted,
        Some("stage-entry budget is exhausted".to_owned()),
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    )
    .expect("retry-exhausted wait");
    let exhausted_id = exhausted.id();
    task.add_wait_condition(exhausted, Timestamp::now_utc())
        .expect("add retry-exhausted wait");

    let result = task.resolve_wait_condition(exhausted_id, Timestamp::now_utc());

    assert!(result.is_err());
    assert_eq!(task.lifecycle(), crate::LifecycleStatus::Waiting);

    let catalog = CancellationReasonCatalog::new([
        CancellationReason::unspecified().expect("mandatory cancellation reason")
    ])
    .expect("catalog");
    task.cancel(
        &catalog,
        CancellationRequest {
            reason_id: Some(
                crate::CancellationReasonId::new(crate::CancellationReasonId::UNSPECIFIED)
                    .expect("reason id"),
            ),
            note: Some("retry budget exhausted".to_owned()),
            cancelled_by: Actor::human(ActorId::new()),
            cancelled_at: Timestamp::now_utc(),
        },
    )
    .expect("cancellation is the M0 retry-exhausted exit");

    assert_eq!(task.lifecycle(), crate::LifecycleStatus::Cancelled);
    assert_eq!(task.wait_conditions().count(), 0);
}

#[test]
fn terminal_task_rejects_further_artifact_attachment() {
    let mut task = draft(TaskKind::Delivery);
    approve_and_start(&mut task);
    task.complete(CompletionData::new(
        CompletionSource::StageOutcome,
        [],
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    ))
    .expect("complete delivery task");
    let artifact = Artifact::new(
        ArtifactId::new(),
        NewArtifact {
            project_id: task.project_id(),
            kind: ArtifactKind::new(ArtifactKind::PLAN).expect("valid kind"),
            title: "Late plan".to_owned(),
            body: ArtifactBody::inline_json(json!({})),
            metadata: json!({}),
            created_by: Actor::human(ActorId::new()),
            created_at: Timestamp::now_utc(),
        },
    )
    .expect("valid artifact");
    let result = task.attach_artifact(
        &artifact,
        ArtifactProducer::Human,
        Actor::human(ActorId::new()),
        None,
        Timestamp::now_utc(),
    );

    assert!(result.is_err());
}
