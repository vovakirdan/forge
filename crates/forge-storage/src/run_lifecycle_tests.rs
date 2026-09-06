use super::{
    FencedWrite, RunDesiredState, RunObservedState, SequenceDecision,
    desired_state_after_observation, executor_submission_is_permitted,
    observed_state_transition_is_monotonic, sequence_decision,
};
use forge_domain::Timestamp;
use time::OffsetDateTime;

#[test]
fn run_sequence_accepts_only_its_exact_next_value() {
    let decision = sequence_decision(4, 5).expect("sequence decision");

    assert_eq!(decision, SequenceDecision::Apply);
}

#[test]
fn run_sequence_reports_a_gap_without_advancing() {
    let decision = sequence_decision(4, 6).expect("sequence decision");

    assert_eq!(
        decision,
        SequenceDecision::Gap {
            expected: 5,
            received: 6,
        }
    );
}

#[test]
fn run_sequence_ignores_an_already_accepted_value() {
    let decision = sequence_decision(4, 4).expect("sequence decision");

    assert_eq!(decision, SequenceDecision::Ignored);
}

#[test]
fn running_observation_confirms_core_running_intent() {
    let desired = desired_state_after_observation(
        RunDesiredState::ProvisionRequested,
        RunObservedState::Running,
    );

    assert_eq!(desired, RunDesiredState::Running);
}

#[test]
fn late_running_observation_preserves_graceful_stop_intent() {
    let desired =
        desired_state_after_observation(RunDesiredState::StopRequested, RunObservedState::Running);

    assert_eq!(desired, RunDesiredState::StopRequested);
}

#[test]
fn late_running_observation_preserves_forced_stop_intent() {
    let desired = desired_state_after_observation(
        RunDesiredState::ForceStopRequested,
        RunObservedState::Running,
    );

    assert_eq!(desired, RunDesiredState::ForceStopRequested);
}

#[test]
fn stopped_observation_records_terminal_core_intent() {
    let desired =
        desired_state_after_observation(RunDesiredState::StopRequested, RunObservedState::Stopped);

    assert_eq!(desired, RunDesiredState::Stopped);
}

#[test]
fn terminal_observation_cannot_return_to_running() {
    let allowed = observed_state_transition_is_monotonic(
        RunObservedState::Stopped,
        RunObservedState::Running,
    );

    assert!(!allowed);
}

#[test]
fn failed_observation_cannot_return_to_running() {
    let allowed =
        observed_state_transition_is_monotonic(RunObservedState::Failed, RunObservedState::Running);

    assert!(!allowed);
}

#[test]
fn failure_can_gain_later_positive_quiescence_evidence() {
    let allowed =
        observed_state_transition_is_monotonic(RunObservedState::Failed, RunObservedState::Stopped);

    assert!(allowed);
}

#[test]
fn observed_state_can_skip_to_a_confirmed_terminal_stop() {
    let allowed = observed_state_transition_is_monotonic(
        RunObservedState::Provisioning,
        RunObservedState::Stopped,
    );

    assert!(allowed);
}

#[test]
fn fenced_gap_exposes_the_recovery_sequence() {
    let write = FencedWrite::SequenceGap {
        expected: 5,
        received: 7,
    };

    assert!(matches!(
        write,
        FencedWrite::SequenceGap {
            expected: 5,
            received: 7
        }
    ));
}

#[test]
fn stopped_project_rejects_executor_submissions() {
    let allowed = executor_submission_is_permitted(false, RunDesiredState::Running);

    assert!(!allowed);
}

#[test]
fn stop_requested_run_rejects_executor_submissions() {
    let allowed = executor_submission_is_permitted(true, RunDesiredState::StopRequested);

    assert!(!allowed);
}

#[test]
fn canonical_run_time_uses_core_receipt_not_supervisor_report() {
    let reported_at = Timestamp::from_offset_date_time(
        OffsetDateTime::from_unix_timestamp(1).expect("reported timestamp"),
    );
    let received_at = Timestamp::from_offset_date_time(
        OffsetDateTime::from_unix_timestamp(2).expect("received timestamp"),
    );
    let update = super::ObservedRunUpdate {
        run_id: uuid::Uuid::now_v7(),
        lease_fencing_token: 1,
        environment_epoch: 1,
        sequence: 1,
        observed_state: RunObservedState::Running,
        details: serde_json::json!({}),
        reported_at,
        received_at,
    };

    assert_eq!(super::canonical_observation_time(&update), received_at);
}
