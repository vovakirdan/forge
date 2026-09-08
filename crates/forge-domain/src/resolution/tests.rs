use super::*;
use crate::{Actor, ActorId, ArtifactId, ProjectId, Timestamp};
use uuid::Uuid;

#[test]
fn answer_recommendation_never_invents_a_pipeline_outcome() {
    let mut answer = ResolutionAnswer {
        disposition: ResolutionDisposition::ContinueStage,
        summary: "Use the documented approach".into(),
        recommended_outcome_key: Some("planning".into()),
    };
    assert!(answer.validate(&["accepted".into()]).is_err());
    answer.recommended_outcome_key = Some("accepted".into());
    assert!(answer.validate(&["accepted".into()]).is_ok());
    answer.summary = "\0".into();
    assert!(answer.validate(&["accepted".into()]).is_err());
}

#[test]
fn assignment_has_one_terminal_answer_and_a_management_lease() {
    let now = Timestamp::now_utc();
    let mut assignment = ResolutionAssignment {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        escalation_id: Uuid::now_v7(),
        resolver: Resolver::Human,
        lease: ResolutionLease {
            generation: 1,
            held_by: Actor::system_manager(ActorId::new()),
            expires_at: None,
        },
        allowed_outcomes: vec!["success".into()],
        issued_at: now,
        state: ResolutionAssignmentState::Active,
    };
    assert!(assignment.validate_snapshot().is_ok());
    assignment
        .finish(ResolutionAssignmentState::Answered {
            artifact_id: ArtifactId::new(),
        })
        .unwrap();
    assert!(
        assignment
            .finish(ResolutionAssignmentState::Retired {
                reason: "late".into()
            })
            .is_err()
    );
}

#[test]
fn route_rejects_duplicate_candidates_and_unbounded_deadlines() {
    let employee = crate::EmployeeId::new();
    let mut route = ResolverRoute {
        project_id: ProjectId::new(),
        key: "engineering".into(),
        revision: 1,
        employee_ids: vec![employee, employee],
        assignment_timeout_seconds: 60,
        updated_at: Timestamp::now_utc(),
    };
    assert!(route.validate_snapshot().is_err());
    route.employee_ids.pop();
    assert!(route.validate_snapshot().is_ok());
    route.assignment_timeout_seconds = 0;
    assert!(route.validate_snapshot().is_err());
}

#[test]
fn communication_source_has_exact_run_scope_and_no_task_authority() {
    let source = EscalationSource::Communication(CommunicationEscalationSource {
        assignment: crate::CommunicationAssignmentRef {
            assignment_id: Uuid::now_v7(),
            thread_id: Uuid::now_v7(),
            source_message_id: Uuid::now_v7(),
        },
        run_id: Uuid::now_v7(),
        fencing_token: 1,
        environment_epoch: 1,
    });
    assert!(source.validate().is_ok());
    assert!(source.task().is_none());
    let mut encoded = serde_json::to_value(&source).unwrap();
    encoded["task_id"] = serde_json::json!(crate::TaskId::new());
    assert!(serde_json::from_value::<EscalationSource>(encoded).is_err());
    let mut invalid = source.communication().unwrap().clone();
    invalid.fencing_token = 0;
    assert!(EscalationSource::Communication(invalid).validate().is_err());
}

#[test]
fn task_question_rejects_zero_stage_visit_during_deserialization() {
    let source = EscalationSource::Task(TaskEscalationSource {
        task_id: crate::TaskId::new(),
        pipeline_version_id: crate::PipelineVersionId::new(),
        stage_id: crate::StageId::new("work").unwrap(),
        stage_visit: crate::StageVisit::INITIAL,
        wait_condition_id: crate::WaitConditionId::new(),
    });
    let mut encoded = serde_json::to_value(source).unwrap();
    encoded["stage_visit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<EscalationSource>(encoded).is_err());
}
