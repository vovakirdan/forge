use std::collections::BTreeSet;

use super::{
    ArtifactRequirement, ArtifactRequirementScope, ExecutorKind, OutcomeKey,
    PipelineTransitionTarget, PipelineVersion, PipelineVersionInput, StageOutcomeSubmission,
    artifact, cancellation_catalog, new_task, stage, transition_with_requirements,
};
use crate::{
    Actor, ActorId, ArtifactKind, ArtifactProducer, LifecycleStatus, PipelineId, PipelineVersionId,
    ProjectId, StageId, TaskKind, TaskWaitCondition, Timestamp, WaitConditionId,
};

#[test]
fn analysis_evidence_can_be_explicitly_cited_by_a_later_human_verdict() {
    let project_id = ProjectId::new();
    let pipeline_id = PipelineId::new();
    let version = PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id,
        project_id,
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Analysis]),
        entry_stage_id: StageId::new("investigate").expect("entry stage"),
        max_stage_visits: None,
        stages: vec![
            stage(
                "investigate",
                ExecutorKind::Employee,
                vec![transition_with_requirements(
                    "researched",
                    PipelineTransitionTarget::Stage(
                        StageId::new("human_verdict").expect("human stage"),
                    ),
                    vec![
                        ArtifactRequirement::new(
                            ArtifactKind::new(ArtifactKind::ANALYSIS_RESULT).expect("kind"),
                            1,
                        )
                        .expect("requirement"),
                    ],
                )],
            ),
            stage(
                "human_verdict",
                ExecutorKind::Human,
                vec![transition_with_requirements(
                    "accept",
                    PipelineTransitionTarget::Done,
                    vec![
                        ArtifactRequirement::new_with_scope(
                            ArtifactKind::new(ArtifactKind::ANALYSIS_RESULT).expect("kind"),
                            1,
                            ArtifactRequirementScope::TaskHistory,
                        )
                        .expect("requirement"),
                        ArtifactRequirement::new(
                            ArtifactKind::new(ArtifactKind::DECISION_RECORD).expect("kind"),
                            1,
                        )
                        .expect("requirement"),
                    ],
                )],
            ),
        ],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    })
    .expect("pipeline version");
    let mut task = new_task(&version, TaskKind::Analysis, "investigate");
    version
        .enter_entry_stage(&mut task, None, Timestamp::now_utc())
        .expect("enter employee stage");
    version
        .start_entry_employee_stage(&mut task, Timestamp::now_utc())
        .expect("start employee stage");

    let analysis_result = artifact(&task, ArtifactKind::ANALYSIS_RESULT, "Investigation");
    task.attach_artifact(
        &analysis_result,
        ArtifactProducer::EmployeeRun,
        Actor::employee(ActorId::new()),
        None,
        Timestamp::now_utc(),
    )
    .expect("attach investigation");
    let next_visit = task
        .current_stage_visit()
        .expect("current visit")
        .next()
        .expect("next visit");
    let verdict_wait = TaskWaitCondition::for_stage(
        WaitConditionId::new(),
        StageId::new("human_verdict").expect("human stage"),
        next_visit,
        None,
        Actor::human(ActorId::new()),
        Timestamp::now_utc(),
    )
    .expect("verdict wait");
    version
        .apply_outcome(
            &mut task,
            StageOutcomeSubmission {
                outcome: OutcomeKey::new("researched").expect("outcome"),
                outcome_artifact_ids: BTreeSet::from([analysis_result.id()]),
                submitted_by: Actor::employee(ActorId::new()),
                submitted_at: Timestamp::now_utc(),
                cancellation_reason_id: None,
                cancellation_note: None,
            },
            &cancellation_catalog(),
            None,
            Some(verdict_wait),
        )
        .expect("enter human verdict");
    assert_eq!(task.lifecycle(), LifecycleStatus::Waiting);

    let decision = artifact(&task, ArtifactKind::DECISION_RECORD, "Accept analysis");
    task.attach_artifact(
        &decision,
        ArtifactProducer::Human,
        Actor::human(ActorId::new()),
        None,
        Timestamp::now_utc(),
    )
    .expect("attach decision");
    let wait_id = task.wait_conditions().next().expect("stage wait").id();
    version
        .apply_outcome(
            &mut task,
            StageOutcomeSubmission {
                outcome: OutcomeKey::new("accept").expect("outcome"),
                outcome_artifact_ids: BTreeSet::from([analysis_result.id(), decision.id()]),
                submitted_by: Actor::human(ActorId::new()),
                submitted_at: Timestamp::now_utc(),
                cancellation_reason_id: None,
                cancellation_note: None,
            },
            &cancellation_catalog(),
            Some(wait_id),
            None,
        )
        .expect("complete analysis");

    assert_eq!(task.lifecycle(), LifecycleStatus::Done);
}
