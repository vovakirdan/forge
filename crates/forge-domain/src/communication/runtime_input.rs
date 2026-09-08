//! Typed controls distinguish model input, transport closure, and observed receipt.
use super::EmployeeMessage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeInputAction {
    Message { source: Box<EmployeeMessage> },
    CloseAfterTurn { reason: RuntimeInputCloseReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInputCloseReason {
    AssignmentCompleted,
    AuthorityLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInputOutcome {
    RuntimeAccepted,
    InputClosed,
    Unsupported,
    StaleScope,
    Conflict,
    DeliveryUnknown,
}
