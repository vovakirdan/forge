use super::*;
use crate::{
    Actor, ActorId, PipelineId, PipelineTransition, PipelineTransitionTarget, PipelineVersionInput,
    TaskKind,
};
fn stage() -> PipelineStage {
    PipelineStage::new(
        StageId::new("publish_here").unwrap(),
        "Owner named action",
        ExecutorKind::System,
        [
            PipelineTransition::new(
                OutcomeKey::new("landed").unwrap(),
                PipelineTransitionTarget::Done,
                vec![],
            )
            .unwrap(),
            PipelineTransition::new(
                OutcomeKey::new("nothing").unwrap(),
                PipelineTransitionTarget::Done,
                vec![],
            )
            .unwrap(),
            PipelineTransition::new(
                OutcomeKey::new("refresh").unwrap(),
                PipelineTransitionTarget::Stage(StageId::new("work").unwrap()),
                vec![],
            )
            .unwrap(),
        ],
    )
    .unwrap()
}
fn action() -> SystemStageAction {
    SystemStageAction::GitIntegration {
        outcomes: IntegrationOutcomes {
            applied: OutcomeKey::new("landed").unwrap(),
            no_changes: OutcomeKey::new("nothing").unwrap(),
            stale_base: OutcomeKey::new("refresh").unwrap(),
        },
        required_review_stages: BTreeSet::new(),
    }
}
fn version(stage: PipelineStage) -> Result<PipelineVersion, DomainError> {
    let work = PipelineStage::new(
        StageId::new("work").unwrap(),
        "Implementation",
        ExecutorKind::Employee,
        [PipelineTransition::new(
            OutcomeKey::new("ready").unwrap(),
            PipelineTransitionTarget::Stage(stage.id().clone()),
            vec![],
        )
        .unwrap()],
    )
    .unwrap();
    PipelineVersion::new(PipelineVersionInput {
        id: PipelineVersionId::new(),
        pipeline_id: PipelineId::new(),
        project_id: ProjectId::new(),
        version: 1,
        task_kinds: BTreeSet::from([TaskKind::Delivery]),
        entry_stage_id: work.id().clone(),
        max_stage_visits: Some(8),
        stages: vec![work, stage],
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
    })
}
#[test]
fn action_is_registered_by_kind_and_exact_outcomes_not_stage_name() {
    let stage = stage().with_system_action(Some(action())).unwrap();
    let version = version(stage).unwrap();
    version.validate_snapshot().unwrap();
    let mut wrong = action();
    let SystemStageAction::GitIntegration { outcomes, .. } = &mut wrong else {
        panic!("wrong action");
    };
    outcomes.applied = OutcomeKey::new("unconfigured").unwrap();
    assert!(self::stage().with_system_action(Some(wrong)).is_err());
}
#[test]
fn review_gate_ids_must_name_real_review_policies_and_old_none_stays_none() {
    assert!(stage().system_action().is_none());
    let mut action = action();
    let SystemStageAction::GitIntegration {
        required_review_stages,
        ..
    } = &mut action
    else {
        panic!("wrong action");
    };
    required_review_stages.insert(StageId::new("work").unwrap());
    assert!(version(stage().with_system_action(Some(action)).unwrap()).is_err());
}

#[test]
fn initial_publication_requires_explicit_unborn_task_identity() {
    use crate::git::{
        GitBranchRef, GitInitialRevision, GitObjectFormat, GitObjectId, LocalGitPath,
    };
    let candidate = GitCandidate {
        commit: GitObjectId::new("a".repeat(40)).unwrap(),
        tree: GitObjectId::new("b".repeat(40)).unwrap(),
    };
    let mut operation = GitIntegrationOperation {
        id: Uuid::now_v7(),
        fence: 1,
        project_id: ProjectId::new(),
        task_id: TaskId::new(),
        pipeline_version_id: PipelineVersionId::new(),
        stage_id: StageId::new("publish_here").unwrap(),
        stage_visit: 1,
        task_revision: 3,
        candidate_proposal_id: Uuid::now_v7(),
        candidate: candidate.clone(),
        binding: TaskGitBinding {
            repository_id: Uuid::now_v7(),
            surface_id: Uuid::now_v7(),
            source: LocalGitPath::new("/tmp/source").unwrap(),
            target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
            initial_base: GitInitialRevision::Unborn {
                object_format: GitObjectFormat::Sha1,
            },
        },
        writer: RunScope {
            run_id: Uuid::now_v7(),
            fencing_token: 1,
            environment_epoch: 1,
        },
        created_at: Timestamp::now_utc(),
    };
    let intent = GitIntegrationIntent {
        operation_id: operation.id,
        target_repository: operation.binding.source.clone(),
        target_ref: operation.binding.target_ref.clone(),
        expected_target: None,
        merge_commit: candidate.commit.clone(),
        candidate,
    };
    operation.validate().unwrap();
    assert!(operation.validate_intent(&intent).is_ok());
    operation.binding.initial_base = GitObjectId::new("c".repeat(40)).unwrap().into();
    assert!(operation.validate_intent(&intent).is_err());
    operation.binding.initial_base = GitInitialRevision::Unborn {
        object_format: GitObjectFormat::Sha256,
    };
    assert!(operation.validate_intent(&intent).is_err());
}
