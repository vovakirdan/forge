use forge_storage::{FencedWrite, RunObservedState};

use super::{InboundResult, refused_fenced_write};

#[test]
fn sequence_gap_is_rejected_instead_of_silently_ignored() {
    let result = refused_fenced_write(FencedWrite::SequenceGap {
        expected: 2,
        received: 3,
    });

    assert!(matches!(
        result,
        InboundResult::Rejected("run_sequence_gap", _)
    ));
}

#[test]
fn invalid_state_regression_is_rejected() {
    let result = refused_fenced_write(FencedWrite::InvalidObservedStateTransition {
        from: RunObservedState::Stopped,
        to: RunObservedState::Running,
    });

    assert!(matches!(
        result,
        InboundResult::Rejected("invalid_observed_state_transition", _)
    ));
}
