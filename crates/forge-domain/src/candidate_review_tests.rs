use crate::{candidate_review::*, runtime::SurfaceAccess, *};
use std::collections::BTreeMap;

fn stage() -> PipelineStage {
    PipelineStage::new(
        StageId::new("owner_choice").unwrap(),
        "Any name",
        ExecutorKind::Employee,
        [PipelineTransition::new(
            OutcomeKey::new("looks_good").unwrap(),
            PipelineTransitionTarget::Done,
            vec![],
        )
        .unwrap()],
    )
    .unwrap()
    .with_requirements(
        String::new(),
        Some(StageWorkspaceRequirements {
            kind: StageWorkspaceKind::Git,
            access: SurfaceAccess::ReadOnly,
        }),
    )
    .unwrap()
}
#[test]
fn review_policy_uses_complete_pipeline_outcomes_not_fixed_names() {
    let policy = StageAcceptancePolicy::CandidateReview {
        independent: true,
        verdicts: BTreeMap::from([(
            OutcomeKey::new("looks_good").unwrap(),
            CandidateVerdict::Accepted,
        )]),
    };
    let stage = stage().with_acceptance_policy(Some(policy)).unwrap();
    assert_eq!(
        stage
            .acceptance_policy()
            .unwrap()
            .verdict(&OutcomeKey::new("looks_good").unwrap()),
        Some(CandidateVerdict::Accepted)
    );
    let invalid = StageAcceptancePolicy::CandidateReview {
        independent: true,
        verdicts: BTreeMap::new(),
    };
    assert!(stage.clone().with_acceptance_policy(Some(invalid)).is_err());
    let roundtrip: PipelineStage =
        serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
    assert_eq!(roundtrip, stage);
}
#[test]
fn assessment_requires_explicit_git_readonly_and_independence_defaults_true() {
    let self_review:StageAcceptancePolicy=serde_json::from_value(serde_json::json!({"kind":"candidate_review","independent":false,"verdicts":{"looks_good":"accepted"}})).unwrap();
    assert!(
        self_review
            .validate_task_surface(&TaskWorkSurface::None)
            .is_err(),
        "allowing self-review does not remove candidate ownership requirements"
    );
    let policy: StageAcceptancePolicy = serde_json::from_value(
        serde_json::json!({"kind":"candidate_review","verdicts":{"looks_good":"accepted"}}),
    )
    .unwrap();
    assert!(policy.requires_independence());
    let writable = stage()
        .with_requirements(
            String::new(),
            Some(StageWorkspaceRequirements {
                kind: StageWorkspaceKind::Git,
                access: SurfaceAccess::ReadWrite,
            }),
        )
        .unwrap();
    assert!(
        writable
            .with_acceptance_policy(Some(policy.clone()))
            .is_err()
    );
    let no_workspace = stage().with_requirements(String::new(), None).unwrap();
    assert!(no_workspace.with_acceptance_policy(Some(policy)).is_err());
}
