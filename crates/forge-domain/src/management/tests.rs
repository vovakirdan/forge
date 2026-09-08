use super::*;
use crate::{
    Actor, ActorId, EmployeeId, PipelineVersionId, ProjectId, StageId, StageVisit, TaskId,
    Timestamp,
};
use uuid::Uuid;

fn constraint() -> NextRunEmployeeConstraint {
    NextRunEmployeeConstraint {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        task_id: TaskId::new(),
        pipeline_version_id: PipelineVersionId::new(),
        stage_id: StageId::new("work").expect("stage"),
        stage_visit: StageVisit::INITIAL,
        employee_id: EmployeeId::new(),
        created_by: Actor::human(ActorId::new()),
        created_at: Timestamp::now_utc(),
        state: NextRunConstraintState::Pending,
    }
}

#[test]
fn dispatch_constraint_holds_and_terminal_results_are_monotonic() {
    let mut pending = constraint();
    pending
        .transition(NextRunConstraintState::Blocked {
            reason: ConstraintBlockReason::EmployeeDisabled,
        })
        .expect("hold");
    let before = pending.clone();
    for target in [
        NextRunConstraintState::Pending,
        NextRunConstraintState::Consumed {
            run_id: Uuid::now_v7(),
        },
    ] {
        assert!(pending.transition(target).is_err());
        assert_eq!(pending, before);
    }
    pending
        .transition(NextRunConstraintState::Cancelled {
            reason: ConstraintEndReason::Cleared,
        })
        .expect("clear");
    assert!(!pending.state.is_active());
    assert!(pending.transition(NextRunConstraintState::Pending).is_err());
}

#[test]
fn dispatch_constraint_consumption_validates_before_mutation() {
    let mut pending = constraint();
    let before = pending.clone();
    assert!(
        pending
            .transition(NextRunConstraintState::Consumed {
                run_id: Uuid::nil()
            })
            .is_err()
    );
    assert_eq!(pending, before);
    let id = Uuid::now_v7();
    pending
        .transition(NextRunConstraintState::Consumed { run_id: id })
        .expect("consume");
    assert!(!pending.state.is_active());
    assert!(
        pending
            .transition(NextRunConstraintState::Cancelled {
                reason: ConstraintEndReason::StageLeft
            })
            .is_err()
    );
}

#[test]
fn resume_schedule_has_one_result_and_a_causal_audit_time() {
    let scope = constraint();
    let mut schedule = TaskResumeSchedule {
        id: scope.id,
        project_id: scope.project_id,
        task_id: scope.task_id,
        expected_task_revision: 3,
        pipeline_version_id: scope.pipeline_version_id,
        stage_id: scope.stage_id,
        stage_visit: scope.stage_visit,
        wait_condition_id: crate::WaitConditionId::new(),
        not_before: scope.created_at,
        reason: "Owner requested continuation".into(),
        created_by: scope.created_by,
        created_at: scope.created_at,
        state: ScheduledResumeState::Pending,
    };
    let before = schedule.clone();
    let earlier = Timestamp::from_offset_date_time(
        scope.created_at.as_offset_date_time() - time::Duration::seconds(1),
    );
    assert!(
        schedule
            .resolve(ScheduledResumeState::Cancelled { at: earlier })
            .is_err()
    );
    assert_eq!(schedule, before);
    schedule
        .resolve(ScheduledResumeState::Cancelled {
            at: scope.created_at,
        })
        .expect("cancel");
    assert!(
        schedule
            .resolve(ScheduledResumeState::Applied {
                command_id: crate::CommandId::new(),
                at: scope.created_at
            })
            .is_err()
    );
}
